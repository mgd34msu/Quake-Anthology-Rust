//! Gyro aiming, calibration, and sensitivity settings menu.
//!
//! Donor provenance: `src/ui/settings/gyro.ts` in full
//! (`registerGyroSettingsMenu`). Tuning reuses the settings-root
//! [`GyroTuningView`](super::GyroTuningView), which carries the full donor
//! gamepad gyro shape; calibration and routing stay behind UI-local views so
//! no input handle crosses into UI code.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::identity::SeatId;

use super::{GyroTuningView, SettingsMenus};
use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatCallback, SeatUiController as _, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};

/// Build a control id from a static template; the templates below always
/// carry the `ui:` namespace and a name part, so a failure is a programming
/// bug.
fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// Build a menu id from a static template; the templates below always carry
/// the `menu:` namespace and a scope part, so a failure is a programming bug.
fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI menu id is invalid: {text}"),
    }
}

/// Calibration snapshot behind the gyro rows (donor `GyroCalibrationState`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GyroCalibrationView {
    /// No calibration in progress and no bias stored.
    Idle,
    /// Calibration in progress.
    Calibrating {
        /// Calibration progress in `[0, 1]`.
        progress: f32,
    },
    /// A bias is stored.
    Ready,
}

/// Gyro route outcome (donor `ControllerOperationResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GyroRouteResult {
    /// The route accepted the change.
    Accepted,
    /// The route rejected the change.
    Rejected {
        /// Human-readable reason shown on the status row.
        reason: String,
    },
}

/// Input-router surface behind the gyro rows.
pub trait GyroRouter {
    /// Read one seat's gyro tuning.
    fn gyro_tuning(&self, seat: SeatId) -> GyroTuningView;
    /// Replace one seat's gyro tuning.
    fn set_gyro_tuning(&mut self, seat: SeatId, tuning: GyroTuningView);
    /// Enable or disable gyro aiming on one seat.
    fn set_gyro_enabled(&mut self, seat: SeatId, enabled: bool) -> GyroRouteResult;
    /// Read one seat's calibration state.
    fn calibration(&self, seat: SeatId) -> GyroCalibrationView;
    /// Begin calibration on one seat.
    fn begin_calibration(&mut self, seat: SeatId) -> GyroRouteResult;
    /// Cancel calibration on one seat.
    fn cancel_calibration(&mut self, seat: SeatId);
    /// Clear the stored bias on one seat.
    fn reset_calibration(&mut self, seat: SeatId);
}

/// Connected-controller snapshot behind the gyro rows: display name plus
/// whether the controller exposes a gyroscope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GyroDeviceView {
    /// Controller display name.
    pub name: String,
    /// Whether the controller exposes a gyroscope.
    pub has_gyro: bool,
}

/// Gyro settings inputs.
#[derive(Clone)]
pub struct GyroSettingsUi {
    /// Input-router surface.
    pub router: Rc<RefCell<dyn GyroRouter>>,
    /// Current controller, if any.
    pub device: Rc<dyn Fn() -> Option<GyroDeviceView>>,
    /// Whether a save is still in flight; rows disable while busy.
    pub busy: Rc<dyn Fn() -> bool>,
    /// Saved-state message for the status rows.
    pub message: Rc<dyn Fn() -> String>,
    /// Persist tuning. The donor save is async and reports a rejection on
    /// the status row; here the error returns synchronously and lands on the
    /// status row immediately.
    pub save: Rc<dyn Fn() -> Result<(), String>>,
}

/// Shared handles behind one gyro menu build.
struct GyroMenuDeps {
    root: UiMenuId,
    title: String,
    seat: SeatId,
    router: Rc<RefCell<dyn GyroRouter>>,
    device: Rc<dyn Fn() -> Option<GyroDeviceView>>,
    busy: Rc<dyn Fn() -> bool>,
    message: Rc<dyn Fn() -> String>,
    error: Rc<RefCell<String>>,
    save: Rc<dyn Fn()>,
    close: Rc<dyn Fn()>,
}

/// Run one save, reporting a failure on the status row. Success leaves a
/// previous error untouched, mirroring the donor catch.
fn report_save(save: &Rc<dyn Fn() -> Result<(), String>>, error: &Rc<RefCell<String>>) {
    if let Err(message) = save() {
        *error.borrow_mut() = message;
    }
}

/// Build the gyro menu: device, enable toggle, calibrate/cancel, status,
/// reset, reconnect hint, saved message, yaw/pitch sliders, back.
fn build_gyro_menu(deps: &GyroMenuDeps) -> UiMenu {
    let rows = MenuRowOptions::default();
    let device = (deps.device)();
    let tuning = deps.router.borrow().gyro_tuning(deps.seat.clone());
    let calibration = deps.router.borrow().calibration(deps.seat.clone());
    let capable = device.as_ref().is_some_and(|device| device.has_gyro);
    let enabled = capable && !(deps.busy)();
    let calibrating = matches!(calibration, GyroCalibrationView::Calibrating { .. });
    let status = if device.is_none() {
        "No controller connected.".to_string()
    } else if !capable {
        "This controller has no gyroscope.".to_string()
    } else {
        match calibration {
            GyroCalibrationView::Calibrating { progress } => {
                format!("Keep still: {}%", (progress * 100.0).round() as i32)
            }
            GyroCalibrationView::Ready => "Calibration complete.".to_string(),
            GyroCalibrationView::Idle => "Place the controller on a steady surface.".to_string(),
        }
    };
    let status = {
        let failed = deps.error.borrow();
        if failed.is_empty() {
            status
        } else {
            failed.clone()
        }
    };
    let button = |id: &str, label: String, row: i32, enabled: bool, on_activate: SeatCallback| UiControl {
        id: control_id(id),
        label,
        rect: menu_row(row, &rows),
        enabled,
        visible: true,
        kind: UiControlKind::Button { on_activate },
    };
    let device_label = device
        .as_ref()
        .map(|device| device.name.chars().take(50).collect::<String>())
        .unwrap_or_else(|| "Controller".to_string());
    let no_activate: SeatCallback = Rc::new(|_| {});
    let toggle_router = Rc::clone(&deps.router);
    let toggle_seat = deps.seat.clone();
    let toggle_error = Rc::clone(&deps.error);
    let toggle_save = Rc::clone(&deps.save);
    let toggle = UiControl {
        id: control_id("ui:gyro:enabled"),
        label: "Gyro aiming".to_string(),
        rect: menu_row(1, &rows),
        enabled,
        visible: true,
        kind: UiControlKind::Toggle {
            checked: tuning.enabled,
            on_change: Rc::new(move |_, value| {
                match toggle_router.borrow_mut().set_gyro_enabled(toggle_seat.clone(), value) {
                    GyroRouteResult::Accepted => {
                        *toggle_error.borrow_mut() = String::new();
                        toggle_save();
                    }
                    GyroRouteResult::Rejected { reason } => {
                        *toggle_error.borrow_mut() = reason;
                    }
                }
            }),
        },
    };
    let calibrate_router = Rc::clone(&deps.router);
    let calibrate_seat = deps.seat.clone();
    let calibrate_error = Rc::clone(&deps.error);
    let calibrate = button(
        "ui:gyro:calibrate",
        (if calibration == GyroCalibrationView::Ready {
            "Recalibrate"
        } else {
            "Calibrate"
        })
        .to_string(),
        2,
        enabled && !calibrating,
        Rc::new(
            move |_| match calibrate_router.borrow_mut().begin_calibration(calibrate_seat.clone()) {
                GyroRouteResult::Accepted => {
                    *calibrate_error.borrow_mut() = String::new();
                }
                GyroRouteResult::Rejected { reason } => {
                    *calibrate_error.borrow_mut() = reason;
                }
            },
        ),
    );
    let cancel_router = Rc::clone(&deps.router);
    let cancel_seat = deps.seat.clone();
    let cancel = button(
        "ui:gyro:cancel",
        "Cancel calibration".to_string(),
        3,
        calibrating,
        Rc::new(move |_| {
            cancel_router.borrow_mut().cancel_calibration(cancel_seat.clone());
        }),
    );
    let reset_router = Rc::clone(&deps.router);
    let reset_seat = deps.seat.clone();
    let reset_error = Rc::clone(&deps.error);
    let reset = button(
        "ui:gyro:reset",
        "Reset calibration".to_string(),
        5,
        enabled,
        Rc::new(move |_| {
            reset_router.borrow_mut().reset_calibration(reset_seat.clone());
            *reset_error.borrow_mut() = String::new();
        }),
    );
    let sensitivity = |id: &str, label: &str, row: i32, value: f32, yaw: bool| {
        let slider_router = Rc::clone(&deps.router);
        let slider_seat = deps.seat.clone();
        let slider_save = Rc::clone(&deps.save);
        UiControl {
            id: control_id(id),
            label: label.to_string(),
            rect: menu_row(row, &rows),
            enabled,
            visible: true,
            kind: UiControlKind::Slider {
                minimum: 0.0,
                maximum: 10.0,
                step: 0.1,
                value,
                value_label: None,
                on_change: Rc::new(move |_, value| {
                    let mut current = slider_router.borrow().gyro_tuning(slider_seat.clone());
                    if yaw {
                        current.yaw_sensitivity = value;
                    } else {
                        current.pitch_sensitivity = value;
                    }
                    slider_router.borrow_mut().set_gyro_tuning(slider_seat.clone(), current);
                    slider_save();
                }),
            },
        }
    };
    let close = Rc::clone(&deps.close);
    let close_router = Rc::clone(&deps.router);
    let close_seat = deps.seat.clone();
    UiMenu {
        scroll: None,
        id: deps.root.clone(),
        title: deps.title.clone(),
        full_screen: false,
        controls: vec![
            button("ui:gyro:device", device_label, 0, false, Rc::clone(&no_activate)),
            toggle,
            calibrate,
            cancel,
            button("ui:gyro:status", status, 4, false, Rc::clone(&no_activate)),
            reset,
            button(
                "ui:gyro:reconnect",
                "Recalibrate after reconnecting.".to_string(),
                6,
                false,
                Rc::clone(&no_activate),
            ),
            button("ui:gyro:saved", (deps.message)(), 7, false, Rc::clone(&no_activate)),
            sensitivity(
                "ui:gyro:yawSensitivity",
                "Gyro yaw sensitivity",
                8,
                tuning.yaw_sensitivity,
                true,
            ),
            sensitivity(
                "ui:gyro:pitchSensitivity",
                "Gyro pitch sensitivity",
                9,
                tuning.pitch_sensitivity,
                false,
            ),
            button("ui:gyro:back", "Back".to_string(), 11, true, Rc::new(move |_| close())),
        ],
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(move |_| {
            close_router.borrow_mut().cancel_calibration(close_seat.clone());
        }),
    }
}

/// Register the gyro settings menu.
///
/// The menu factory captures the seat by value; control callbacks re-enter
/// the controller through the shared handle, so callers must not hold a
/// borrow across input or draw calls that activate those controls. Closing
/// the menu cancels calibration, mirroring the donor close hook.
pub fn register_gyro_settings_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    settings: GyroSettingsUi,
    title: Option<&str>,
) -> SettingsMenus {
    let root = menu_id("menu:settings:gyro");
    let seat = controller.borrow().seat();
    let GyroSettingsUi {
        router,
        device,
        busy,
        message,
        save,
    } = settings;
    let error: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
    let report_error = Rc::clone(&error);
    let save_now: Rc<dyn Fn()> = Rc::new(move || report_save(&save, &report_error));
    let back_controller = Rc::clone(controller);
    let deps = GyroMenuDeps {
        root: root.clone(),
        title: title.unwrap_or("Gyro controls").to_string(),
        seat,
        router,
        device,
        busy,
        message,
        error,
        save: save_now,
        close: Rc::new(move || back_controller.borrow_mut().close_menu()),
    };
    controller
        .borrow_mut()
        .register(root.clone(), Rc::new(move || build_gyro_menu(&deps)));
    SettingsMenus::new(controller, root.clone(), vec![root])
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use qa_core::identity::IdentityOwner;

    use super::*;
    use crate::ui::common::controller::headless_options;
    use crate::ui::settings::GyroYawAxis;

    struct FakeRouter {
        tuning: GyroTuningView,
        calibration: GyroCalibrationView,
        enable_result: GyroRouteResult,
        begin_result: GyroRouteResult,
        begins: usize,
        cancels: usize,
        resets: usize,
    }

    impl FakeRouter {
        fn idle() -> Self {
            Self {
                tuning: GyroTuningView {
                    enabled: false,
                    yaw_sensitivity: 1.0,
                    pitch_sensitivity: 1.0,
                    yaw_axis: GyroYawAxis::Y,
                },
                calibration: GyroCalibrationView::Idle,
                enable_result: GyroRouteResult::Accepted,
                begin_result: GyroRouteResult::Accepted,
                begins: 0,
                cancels: 0,
                resets: 0,
            }
        }
    }

    impl GyroRouter for FakeRouter {
        fn gyro_tuning(&self, _seat: SeatId) -> GyroTuningView {
            self.tuning
        }

        fn set_gyro_tuning(&mut self, _seat: SeatId, tuning: GyroTuningView) {
            self.tuning = tuning;
        }

        fn set_gyro_enabled(&mut self, _seat: SeatId, enabled: bool) -> GyroRouteResult {
            match &self.enable_result {
                GyroRouteResult::Accepted => {
                    self.tuning.enabled = enabled;
                    GyroRouteResult::Accepted
                }
                GyroRouteResult::Rejected { reason } => GyroRouteResult::Rejected { reason: reason.clone() },
            }
        }

        fn calibration(&self, _seat: SeatId) -> GyroCalibrationView {
            self.calibration
        }

        fn begin_calibration(&mut self, _seat: SeatId) -> GyroRouteResult {
            self.begins += 1;
            self.begin_result.clone()
        }

        fn cancel_calibration(&mut self, _seat: SeatId) {
            self.cancels += 1;
        }

        fn reset_calibration(&mut self, _seat: SeatId) {
            self.resets += 1;
        }
    }

    fn seat() -> SeatId {
        IdentityOwner::create("gyro-test").unwrap().seat(0)
    }

    fn device(name: &str) -> Option<GyroDeviceView> {
        Some(GyroDeviceView {
            name: name.to_string(),
            has_gyro: true,
        })
    }

    #[allow(clippy::type_complexity)]
    fn harness(
        seat: SeatId,
        device: Option<GyroDeviceView>,
        router: FakeRouter,
    ) -> (GyroMenuDeps, Rc<RefCell<FakeRouter>>, Rc<Cell<usize>>, Rc<Cell<usize>>) {
        let router = Rc::new(RefCell::new(router));
        let router_dyn = Rc::clone(&router) as Rc<RefCell<dyn GyroRouter>>;
        let saves = Rc::new(Cell::new(0));
        let closes = Rc::new(Cell::new(0));
        let save_saves = Rc::clone(&saves);
        let close_closes = Rc::clone(&closes);
        let deps = GyroMenuDeps {
            root: menu_id("menu:settings:gyro"),
            title: "Gyro controls".to_string(),
            seat,
            router: router_dyn,
            device: Rc::new(move || device.clone()),
            busy: Rc::new(|| false),
            message: Rc::new(|| "All settings saved.".to_string()),
            error: Rc::new(RefCell::new(String::new())),
            save: Rc::new(move || save_saves.set(save_saves.get() + 1)),
            close: Rc::new(move || close_closes.set(close_closes.get() + 1)),
        };
        (deps, router, saves, closes)
    }

    fn control<'m>(menu: &'m UiMenu, id: &str) -> &'m UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control {id}"))
    }

    fn activate(menu: &UiMenu, seat: &SeatId, id: &str) {
        let UiControlKind::Button { on_activate } = &control(menu, id).kind else {
            panic!("{id} is a button");
        };
        on_activate(seat.clone());
    }

    fn toggle(menu: &UiMenu, seat: &SeatId, id: &str, value: bool) {
        let UiControlKind::Toggle { on_change, .. } = &control(menu, id).kind else {
            panic!("{id} is a toggle");
        };
        on_change(seat.clone(), value);
    }

    fn slide(menu: &UiMenu, seat: &SeatId, id: &str, value: f32) {
        let UiControlKind::Slider { on_change, .. } = &control(menu, id).kind else {
            panic!("{id} is a slider");
        };
        on_change(seat.clone(), value);
    }

    #[test]
    fn rows_without_device_disable_and_report() {
        let seat = seat();
        let (deps, _, _, _) = harness(seat, None, FakeRouter::idle());
        let menu = build_gyro_menu(&deps);
        assert_eq!(menu.id.as_str(), "menu:settings:gyro");
        assert_eq!(menu.title, "Gyro controls");
        assert!(!menu.full_screen);
        assert_eq!(menu.controls.len(), 11);
        let rows = MenuRowOptions::default();
        for (id, row) in [
            ("ui:gyro:device", 0),
            ("ui:gyro:enabled", 1),
            ("ui:gyro:calibrate", 2),
            ("ui:gyro:cancel", 3),
            ("ui:gyro:status", 4),
            ("ui:gyro:reset", 5),
            ("ui:gyro:reconnect", 6),
            ("ui:gyro:saved", 7),
            ("ui:gyro:yawSensitivity", 8),
            ("ui:gyro:pitchSensitivity", 9),
            ("ui:gyro:back", 11),
        ] {
            assert_eq!(control(&menu, id).rect, menu_row(row, &rows), "{id} row");
        }
        assert_eq!(control(&menu, "ui:gyro:device").label, "Controller");
        assert!(!control(&menu, "ui:gyro:enabled").enabled);
        assert_eq!(control(&menu, "ui:gyro:status").label, "No controller connected.");
        assert_eq!(control(&menu, "ui:gyro:saved").label, "All settings saved.");
        assert_eq!(
            control(&menu, "ui:gyro:reconnect").label,
            "Recalibrate after reconnecting."
        );
        assert!(control(&menu, "ui:gyro:back").enabled);
    }

    #[test]
    fn rows_without_gyro_report_missing_sensor() {
        let seat = seat();
        let (deps, _, _, _) = harness(
            seat,
            Some(GyroDeviceView {
                name: "Pad".to_string(),
                has_gyro: false,
            }),
            FakeRouter::idle(),
        );
        let menu = build_gyro_menu(&deps);
        assert_eq!(
            control(&menu, "ui:gyro:status").label,
            "This controller has no gyroscope."
        );
        assert!(!control(&menu, "ui:gyro:enabled").enabled);
        assert!(!control(&menu, "ui:gyro:calibrate").enabled);
    }

    #[test]
    fn rows_follow_calibration_state() {
        let seat = seat();
        let (deps, _, _, _) = harness(
            seat.clone(),
            device("Pad"),
            FakeRouter {
                calibration: GyroCalibrationView::Calibrating { progress: 0.5 },
                ..FakeRouter::idle()
            },
        );
        let menu = build_gyro_menu(&deps);
        assert_eq!(control(&menu, "ui:gyro:status").label, "Keep still: 50%");
        assert_eq!(control(&menu, "ui:gyro:calibrate").label, "Calibrate");
        assert!(!control(&menu, "ui:gyro:calibrate").enabled);
        assert!(control(&menu, "ui:gyro:cancel").enabled);

        let (deps, _, _, _) = harness(
            seat.clone(),
            device("Pad"),
            FakeRouter {
                calibration: GyroCalibrationView::Ready,
                ..FakeRouter::idle()
            },
        );
        let menu = build_gyro_menu(&deps);
        assert_eq!(control(&menu, "ui:gyro:status").label, "Calibration complete.");
        assert_eq!(control(&menu, "ui:gyro:calibrate").label, "Recalibrate");
        assert!(control(&menu, "ui:gyro:calibrate").enabled);
        assert!(!control(&menu, "ui:gyro:cancel").enabled);

        let (deps, _, _, _) = harness(seat, device("Pad"), FakeRouter::idle());
        let menu = build_gyro_menu(&deps);
        assert_eq!(
            control(&menu, "ui:gyro:status").label,
            "Place the controller on a steady surface."
        );
    }

    #[test]
    fn busy_disables_interactive_rows() {
        let seat = seat();
        let (deps, _, _, _) = harness(seat, device("Pad"), FakeRouter::idle());
        let deps = GyroMenuDeps {
            busy: Rc::new(|| true),
            ..deps
        };
        let menu = build_gyro_menu(&deps);
        assert!(!control(&menu, "ui:gyro:enabled").enabled);
        assert!(!control(&menu, "ui:gyro:calibrate").enabled);
        assert!(!control(&menu, "ui:gyro:reset").enabled);
    }

    #[test]
    fn toggle_accept_saves_and_clears_error() {
        let seat = seat();
        let (deps, router, saves, _) = harness(seat.clone(), device("Pad"), FakeRouter::idle());
        *deps.error.borrow_mut() = "stale".to_string();
        toggle(&build_gyro_menu(&deps), &seat, "ui:gyro:enabled", true);
        assert!(router.borrow().tuning.enabled);
        assert_eq!(saves.get(), 1);
        assert!(deps.error.borrow().is_empty());
        let menu = build_gyro_menu(&deps);
        let UiControlKind::Toggle { checked, .. } = &control(&menu, "ui:gyro:enabled").kind else {
            panic!("enabled is a toggle");
        };
        assert!(*checked);
    }

    #[test]
    fn toggle_reject_reports_reason_without_saving() {
        let seat = seat();
        let (deps, router, saves, _) = harness(
            seat.clone(),
            device("Pad"),
            FakeRouter {
                enable_result: GyroRouteResult::Rejected {
                    reason: "sensor busy".to_string(),
                },
                ..FakeRouter::idle()
            },
        );
        toggle(&build_gyro_menu(&deps), &seat, "ui:gyro:enabled", true);
        assert!(!router.borrow().tuning.enabled);
        assert_eq!(saves.get(), 0);
        assert_eq!(control(&build_gyro_menu(&deps), "ui:gyro:status").label, "sensor busy");
    }

    #[test]
    fn save_failure_reports_status_row() {
        let error: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
        let failing: Rc<dyn Fn() -> Result<(), String>> = Rc::new(|| Err("disk is gone".to_string()));
        report_save(&failing, &error);
        assert_eq!(*error.borrow(), "disk is gone");
        let passing: Rc<dyn Fn() -> Result<(), String>> = Rc::new(|| Ok(()));
        report_save(&passing, &error);
        assert_eq!(*error.borrow(), "disk is gone");
    }

    #[test]
    fn sliders_update_tuning_and_save() {
        let seat = seat();
        let (deps, router, saves, _) = harness(seat.clone(), device("Pad"), FakeRouter::idle());
        let menu = build_gyro_menu(&deps);
        let UiControlKind::Slider {
            minimum,
            maximum,
            step,
            value,
            ..
        } = &control(&menu, "ui:gyro:yawSensitivity").kind
        else {
            panic!("yaw is a slider");
        };
        assert_eq!((*minimum, *maximum, *step, *value), (0.0, 10.0, 0.1, 1.0));
        slide(&menu, &seat, "ui:gyro:yawSensitivity", 2.5);
        slide(&menu, &seat, "ui:gyro:pitchSensitivity", 3.5);
        assert_eq!(router.borrow().tuning.yaw_sensitivity, 2.5);
        assert_eq!(router.borrow().tuning.pitch_sensitivity, 3.5);
        assert_eq!(router.borrow().tuning.yaw_axis, GyroYawAxis::Y);
        assert_eq!(saves.get(), 2);
    }

    #[test]
    fn calibrate_cancel_reset_and_close_flow() {
        let seat = seat();
        let (deps, router, _, _) = harness(seat.clone(), device("Pad"), FakeRouter::idle());
        let menu = build_gyro_menu(&deps);
        activate(&menu, &seat, "ui:gyro:calibrate");
        assert_eq!(router.borrow().begins, 1);
        activate(&menu, &seat, "ui:gyro:cancel");
        assert_eq!(router.borrow().cancels, 1);
        *deps.error.borrow_mut() = "stale".to_string();
        activate(&menu, &seat, "ui:gyro:reset");
        assert_eq!(router.borrow().resets, 1);
        assert!(deps.error.borrow().is_empty());
        (menu.on_close)(seat.clone());
        assert_eq!(router.borrow().cancels, 2);
    }

    #[test]
    fn calibrate_reject_reports_reason() {
        let seat = seat();
        let (deps, router, _, _) = harness(
            seat.clone(),
            device("Pad"),
            FakeRouter {
                begin_result: GyroRouteResult::Rejected {
                    reason: "no sensor".to_string(),
                },
                ..FakeRouter::idle()
            },
        );
        activate(&build_gyro_menu(&deps), &seat, "ui:gyro:calibrate");
        assert_eq!(router.borrow().begins, 1);
        assert_eq!(control(&build_gyro_menu(&deps), "ui:gyro:status").label, "no sensor");
    }

    #[test]
    fn back_button_closes_menu() {
        let seat = seat();
        let (deps, _, _, closes) = harness(seat.clone(), device("Pad"), FakeRouter::idle());
        activate(&build_gyro_menu(&deps), &seat, "ui:gyro:back");
        assert_eq!(closes.get(), 1);
    }

    #[test]
    fn register_open_close_and_dispose() {
        let owner = IdentityOwner::create("gyro-register").unwrap();
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let router = Rc::new(RefCell::new(FakeRouter::idle()));
        let router_dyn = Rc::clone(&router) as Rc<RefCell<dyn GyroRouter>>;
        let menus = register_gyro_settings_menu(
            &controller,
            GyroSettingsUi {
                router: router_dyn,
                device: Rc::new(|| None),
                busy: Rc::new(|| false),
                message: Rc::new(String::new),
                save: Rc::new(|| Ok(())),
            },
            None,
        );
        assert_eq!(menus.root.as_str(), "menu:settings:gyro");
        assert!(controller.borrow().is_registered(&menus.root));
        controller.borrow_mut().open_menu(&menus.root).unwrap();
        assert_eq!(controller.borrow().active_menu().as_ref(), Some(&menus.root));
        controller.borrow_mut().close_menu();
        assert_eq!(router.borrow().cancels, 1);
        menus.dispose();
        assert!(!controller.borrow().is_registered(&menu_id("menu:settings:gyro")));
    }
}
