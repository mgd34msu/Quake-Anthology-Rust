//! Team Arena post-match results menu.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/team-arena-results.ts`
//! (`TeamArenaResultService`, `TeamArenaResults`). Menu construction reuses
//! the real `qa_client` UI surface (`NativeUiController`, `menu_row`,
//! `UiControl`); the donor's borrowed controller/service references become
//! an owned `Rc<RefCell<S>>` service shared with the menu factory plus a
//! `&mut NativeUiController` passed to each call.

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::ui::common::controller::{NativeUiController, UiMenuFactory};
use qa_client::ui::common::layout::{menu_row, MenuRowOptions};
use qa_client::ui::types::{SeatUiController, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};
use thiserror::Error;

/// Results menu id (donor `menu:application:team-arena-results`).
pub const TEAM_ARENA_RESULTS_MENU_ID: &str = "menu:application:team-arena-results";

/// Team Arena results menu failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TeamArenaResultsError {
    /// A static menu or control id was rejected.
    #[error("Team Arena results id is invalid: {0}")]
    BadId(String),
    /// Opening the menu failed.
    #[error("Team Arena results menu failed to open: {0}")]
    OpenFailed(String),
}

/// One completed match result (donor `TeamArenaResultService.read` row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaResult {
    /// Match title.
    pub title: String,
    /// Whether the local player won.
    pub won: bool,
    /// Local score.
    pub score: i32,
    /// Opponent score.
    pub opponent: i32,
    /// Awarded points.
    pub points: i32,
    /// Match time text.
    pub time: String,
}

/// Result backend (donor `TeamArenaResultService`).
pub trait TeamArenaResultService {
    /// Current result, or `None` before the match completes.
    fn read(&self) -> Option<TeamArenaResult>;
    /// Whether the demo button is offered.
    fn demo_available(&self) -> bool {
        false
    }
    /// Play the post-match demo.
    fn play_demo(&mut self) {}
    /// Advance to the next match.
    fn next(&mut self);
    /// Retry the match.
    fn retry(&mut self);
    /// Quit to the main menu.
    fn quit(&mut self);
}

fn button<S: TeamArenaResultService + 'static>(
    service: &Rc<RefCell<S>>,
    id: UiControlId,
    label: String,
    row: i32,
    enabled: bool,
    activate: impl Fn(&mut S) + 'static,
) -> UiControl {
    let service = Rc::clone(service);
    UiControl {
        id,
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| activate(&mut service.borrow_mut())),
        },
    }
}

fn results_menu<S: TeamArenaResultService + 'static>(
    menu_id: &UiMenuId,
    controls: &[(&str, UiControlId)],
    service: &Rc<RefCell<S>>,
) -> UiMenu {
    let result = service.borrow().read();
    let enabled = result.is_some();
    let title = match &result {
        None => "Match complete".to_string(),
        Some(result) => format!(
            "{}: {} ({} - {})",
            if result.won { "Victory" } else { "Defeat" },
            result.title,
            result.score,
            result.opponent
        ),
    };
    let control = |name: &str| {
        controls
            .iter()
            .find(|(id, _)| *id == name)
            .unwrap_or_else(|| panic!("results control {name} was registered"))
            .1
            .clone()
    };
    let mut menu_controls = vec![UiControl {
        id: control("score"),
        label: match &result {
            None => String::new(),
            Some(result) => format!("Score {} \u{b7} Time {}", result.points, result.time),
        },
        rect: menu_row(0, &MenuRowOptions::default()),
        enabled: false,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(|_| {}),
        },
    }];
    menu_controls.push(button(
        service,
        control("next"),
        "Next match".to_string(),
        2,
        enabled,
        S::next,
    ));
    menu_controls.push(button(
        service,
        control("retry"),
        "Retry match".to_string(),
        3,
        enabled,
        S::retry,
    ));
    if service.borrow().demo_available() {
        menu_controls.push(button(
            service,
            control("demo"),
            "Watch demo".to_string(),
            4,
            enabled,
            S::play_demo,
        ));
    }
    menu_controls.push(button(
        service,
        control("main-menu"),
        "Main menu".to_string(),
        5,
        enabled,
        S::quit,
    ));
    UiMenu {
        scroll: None,
        id: menu_id.clone(),
        title,
        full_screen: true,
        controls: menu_controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Registered results menu (donor `TeamArenaResults`).
pub struct TeamArenaResults<S> {
    service: Rc<RefCell<S>>,
    menu_id: UiMenuId,
    active: bool,
}

impl<S: TeamArenaResultService + 'static> TeamArenaResults<S> {
    /// Register the results menu on `controller`.
    pub fn new(controller: &mut NativeUiController, service: S) -> Result<Self, TeamArenaResultsError> {
        let menu_id = UiMenuId::new(TEAM_ARENA_RESULTS_MENU_ID)
            .map_err(|error| TeamArenaResultsError::BadId(error.to_string()))?;
        let mut controls = Vec::new();
        for name in ["score", "next", "retry", "demo", "main-menu"] {
            let id = UiControlId::new(&format!("ui:team-arena:{name}"))
                .map_err(|error| TeamArenaResultsError::BadId(error.to_string()))?;
            controls.push((name, id));
        }
        let shared = Rc::new(RefCell::new(service));
        let factory_service = Rc::clone(&shared);
        let factory_id = menu_id.clone();
        let factory: UiMenuFactory = Rc::new(move || results_menu(&factory_id, &controls, &factory_service));
        controller.register(menu_id.clone(), factory);
        Ok(Self {
            service: shared,
            menu_id,
            active: false,
        })
    }

    /// Whether the results screen has activated.
    #[must_use]
    pub fn active(&self) -> bool {
        self.active
    }

    /// Borrow the shared service.
    #[must_use]
    pub fn service(&self) -> std::cell::Ref<'_, S> {
        self.service.borrow()
    }

    /// Show the results menu once a result is available.
    pub fn update(&mut self, controller: &mut NativeUiController) -> Result<(), TeamArenaResultsError> {
        if self.service.borrow().read().is_none() {
            return Ok(());
        }
        if !self.active {
            self.active = true;
            controller.close_all();
        }
        if controller.active_menu().is_none() {
            controller
                .open_menu(&self.menu_id)
                .map_err(|error| TeamArenaResultsError::OpenFailed(error.to_string()))?;
        }
        Ok(())
    }

    /// Unregister the results menu.
    pub fn close(&self, controller: &mut NativeUiController) {
        controller.unregister(&self.menu_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::ui::common::controller::headless_options;
    use qa_core::identity::IdentityOwner;

    struct FakeService {
        result: Option<TeamArenaResult>,
        demo: bool,
        calls: Vec<String>,
    }

    impl TeamArenaResultService for FakeService {
        fn read(&self) -> Option<TeamArenaResult> {
            self.result.clone()
        }

        fn demo_available(&self) -> bool {
            self.demo
        }

        fn play_demo(&mut self) {
            self.calls.push("demo".to_string());
        }

        fn next(&mut self) {
            self.calls.push("next".to_string());
        }

        fn retry(&mut self) {
            self.calls.push("retry".to_string());
        }

        fn quit(&mut self) {
            self.calls.push("quit".to_string());
        }
    }

    fn controller() -> NativeUiController {
        let owner = IdentityOwner::create("team-arena-results").expect("test identity");
        NativeUiController::new(headless_options(owner.seat(0)))
    }

    fn victory() -> TeamArenaResult {
        TeamArenaResult {
            title: "Arena Gate".to_string(),
            won: true,
            score: 10,
            opponent: 4,
            points: 1250,
            time: "08:32".to_string(),
        }
    }

    #[test]
    fn waits_for_result_before_opening() {
        let mut controller = controller();
        let mut results = TeamArenaResults::new(
            &mut controller,
            FakeService {
                result: None,
                demo: false,
                calls: Vec::new(),
            },
        )
        .unwrap();
        results.update(&mut controller).unwrap();
        assert!(!results.active());
        assert!(controller.active_menu().is_none());
        results.close(&mut controller);
        assert!(!controller.is_registered(&UiMenuId::new(TEAM_ARENA_RESULTS_MENU_ID).unwrap()));
    }

    #[test]
    fn opens_victory_menu_without_demo() {
        let mut controller = controller();
        let mut results = TeamArenaResults::new(
            &mut controller,
            FakeService {
                result: Some(victory()),
                demo: false,
                calls: Vec::new(),
            },
        )
        .unwrap();
        results.update(&mut controller).unwrap();
        assert!(results.active());
        let id = UiMenuId::new(TEAM_ARENA_RESULTS_MENU_ID).unwrap();
        assert_eq!(controller.active_menu(), Some(id));
        // Second update keeps the open menu instead of reopening it.
        results.update(&mut controller).unwrap();
        assert!(results.active());
    }

    #[test]
    fn defeat_menu_offers_demo_and_buttons() {
        let controller = controller();
        let service = Rc::new(RefCell::new(FakeService {
            result: Some(TeamArenaResult {
                won: false,
                ..victory()
            }),
            demo: true,
            calls: Vec::new(),
        }));
        let menu_id = UiMenuId::new(TEAM_ARENA_RESULTS_MENU_ID).unwrap();
        let controls: Vec<(&str, UiControlId)> = ["score", "next", "retry", "demo", "main-menu"]
            .into_iter()
            .map(|name| (name, UiControlId::new(&format!("ui:team-arena:{name}")).unwrap()))
            .collect();
        let menu = results_menu(&menu_id, &controls, &service);
        assert_eq!(menu.title, "Defeat: Arena Gate (10 - 4)");
        assert!(menu.full_screen);
        assert_eq!(menu.controls.len(), 5);
        assert_eq!(menu.controls[0].label, "Score 1250 \u{b7} Time 08:32");
        assert!(!menu.controls[0].enabled);
        assert_eq!(menu.controls[3].label, "Watch demo");
        assert!(menu.controls[3].enabled);

        // Activating each button reaches the service.
        let seat = controller.seat();
        for control in menu.controls.iter().skip(1) {
            let UiControlKind::Button { on_activate } = &control.kind else {
                panic!("expected a button");
            };
            on_activate(seat.clone());
        }
        assert_eq!(
            service.borrow().calls,
            vec![
                "next".to_string(),
                "retry".to_string(),
                "demo".to_string(),
                "quit".to_string()
            ]
        );

        // Without a result the buttons (including the demo) disable.
        service.borrow_mut().result = None;
        service.borrow_mut().demo = true;
        let menu = results_menu(&menu_id, &controls, &service);
        assert_eq!(menu.title, "Match complete");
        assert_eq!(menu.controls[0].label, "");
        assert_eq!(menu.controls.len(), 5);
        assert!(menu.controls.iter().skip(1).all(|control| !control.enabled));
    }
}
