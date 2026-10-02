//! Seat player-death menu (load, restart, or end the game).
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/player-death.ts`
//! (`playerDeathMenu`, `SeatPlayerDeath`). Synchronous port: menus go
//! through [`PlayerDeathController`] (`qa-client`'s controller does not
//! expose menu open yet) and level restart through [`PlayerDeathRecovery`]
//! (the shared save-menu contract omits recovery).

use qa_client::input::KeyCode;
use qa_client::ui::common::layout::{menu_row, MenuRowOptions};
use qa_client::ui::saves::menu::{SaveAction, SavedGameMenuService};
use qa_client::ui::types::{
    SeatInputEvent, SeatInputEventKind, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId,
};
use std::cell::RefCell;
use std::rc::Rc;

/// Menu id for the player-death menu.
pub const PLAYER_DEATH_MENU: &str = "menu:application:death";

/// Menu controller surface used by the death menu.
pub trait PlayerDeathController {
    /// Register the death menu factory.
    fn register_death(&mut self, factory: Rc<dyn Fn() -> UiMenu>);
    /// Open a menu.
    fn open_menu(&mut self, id: &UiMenuId);
    /// Close every menu.
    fn close_all(&mut self);
    /// Active menu, if any.
    fn active_menu(&self) -> Option<UiMenuId>;
    /// Unregister the death menu.
    fn unregister_death(&mut self);
}

/// Level-restart recovery behind the death menu.
pub trait PlayerDeathRecovery {
    /// Restart the level.
    fn restart(&mut self) -> Result<(), String>;
}

struct DeathInner {
    active: bool,
    busy: bool,
    message: String,
    saves: Option<Rc<RefCell<dyn SavedGameMenuService>>>,
    recovery: Option<Rc<RefCell<dyn PlayerDeathRecovery>>>,
    load_menu: UiMenuId,
    quit: Box<dyn FnMut()>,
}

impl DeathInner {
    fn load_enabled(&self) -> bool {
        self.saves
            .as_ref()
            .is_some_and(|saves| saves.borrow().unavailable(SaveAction::Load).is_none())
            && !self.busy
    }

    fn restart(&mut self) {
        let Some(recovery) = self.recovery.clone() else {
            return;
        };
        if self.busy {
            return;
        }
        self.busy = true;
        self.message = "Restarting level...".to_owned();
        let result = recovery.borrow_mut().restart();
        self.busy = false;
        if let Err(error) = result {
            self.message = error;
        }
    }
}

fn death_button(id: &str, label: String, row: i32, enabled: bool, action: Rc<dyn Fn()>) -> UiControl {
    UiControl {
        id: UiControlId::new(&format!("ui:death:{id}")).expect("death control id"),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        visible: true,
        enabled,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| action()),
        },
    }
}

fn death_menu<C: PlayerDeathController + 'static>(
    inner: &Rc<RefCell<DeathInner>>,
    controller: Rc<RefCell<C>>,
) -> UiMenu {
    let state = inner.borrow();
    let mut controls = Vec::new();
    {
        let controller = Rc::clone(&controller);
        let load_menu = state.load_menu.clone();
        controls.push(death_button(
            "load",
            "Load saved game".to_owned(),
            2,
            state.load_enabled(),
            Rc::new(move || controller.borrow_mut().open_menu(&load_menu)),
        ));
    }
    {
        let restarting = Rc::clone(inner);
        controls.push(death_button(
            "restart",
            "Restart level".to_owned(),
            3,
            !state.busy,
            Rc::new(move || restarting.borrow_mut().restart()),
        ));
    }
    {
        let quitting = Rc::clone(inner);
        controls.push(death_button(
            "quit",
            "End game".to_owned(),
            6,
            !state.busy,
            Rc::new(move || (quitting.borrow_mut().quit)()),
        ));
    }
    if !state.message.is_empty() {
        controls.push(death_button("message", state.message.clone(), 8, false, Rc::new(|| {})));
    }
    UiMenu {
        scroll: None,
        id: UiMenuId::new(PLAYER_DEATH_MENU).expect("death menu id"),
        title: "You died".to_owned(),
        full_screen: true,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Seat player-death menu.
pub struct SeatPlayerDeath<C> {
    controller: Rc<RefCell<C>>,
    inner: Rc<RefCell<DeathInner>>,
}

impl<C: PlayerDeathController + 'static> SeatPlayerDeath<C> {
    /// Create the death menu, registering its factory.
    pub fn new(
        controller: C,
        saves: Option<Rc<RefCell<dyn SavedGameMenuService>>>,
        recovery: Option<Rc<RefCell<dyn PlayerDeathRecovery>>>,
        load_menu: UiMenuId,
        quit: Box<dyn FnMut()>,
    ) -> Self {
        let controller = Rc::new(RefCell::new(controller));
        let inner = Rc::new(RefCell::new(DeathInner {
            active: false,
            busy: false,
            message: String::new(),
            saves,
            recovery,
            load_menu,
            quit,
        }));
        let factory_inner = Rc::clone(&inner);
        let factory_controller = Rc::clone(&controller);
        controller.borrow_mut().register_death(Rc::new(move || {
            death_menu(&factory_inner, Rc::clone(&factory_controller))
        }));
        Self { controller, inner }
    }

    /// Whether the death flow is active.
    #[must_use]
    pub fn active(&self) -> bool {
        self.inner.borrow().active
    }

    /// Observe seat health, opening or closing the death menu.
    pub fn observe(&mut self, health: f64) {
        if self.inner.borrow().recovery.is_none() {
            return;
        }
        if health > 0.0 {
            if self.inner.borrow().active {
                self.controller.borrow_mut().close_all();
            }
            self.inner.borrow_mut().active = false;
            return;
        }
        if !self.inner.borrow().active {
            self.controller.borrow_mut().close_all();
            self.inner.borrow_mut().active = true;
        }
        if self.controller.borrow().active_menu().is_none() {
            let death = UiMenuId::new(PLAYER_DEATH_MENU).expect("death menu id");
            self.controller.borrow_mut().open_menu(&death);
        }
    }

    /// Handle one seat input event; returns whether it was consumed.
    #[must_use]
    pub fn input(&self, event: &SeatInputEvent) -> bool {
        if !self.inner.borrow().active {
            return false;
        }
        let death = UiMenuId::new(PLAYER_DEATH_MENU).expect("death menu id");
        if self.controller.borrow().active_menu().as_ref() != Some(&death) {
            return false;
        }
        match &event.kind {
            SeatInputEventKind::Key { code, .. } => *code == KeyCode::Escape as i32,
            SeatInputEventKind::ControllerButton { button, .. } => *button == 1 || *button == 6,
            SeatInputEventKind::MouseButton { button, .. } => *button == 3,
            _ => false,
        }
    }

    /// Unregister the death menu.
    pub fn close(self) {
        self.controller.borrow_mut().unregister_death();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_client::ui::saves::menu::SaveListView;
    use qa_core::identity::IdentityOwner;

    struct FakeController {
        factory: Option<Rc<dyn Fn() -> UiMenu>>,
        active: Option<UiMenuId>,
        opens: Vec<String>,
        closes: usize,
    }

    impl PlayerDeathController for FakeController {
        fn register_death(&mut self, factory: Rc<dyn Fn() -> UiMenu>) {
            self.factory = Some(factory);
        }

        fn open_menu(&mut self, id: &UiMenuId) {
            self.opens.push(id.as_str().to_owned());
            self.active = Some(id.clone());
        }

        fn close_all(&mut self) {
            self.closes += 1;
            self.active = None;
        }

        fn active_menu(&self) -> Option<UiMenuId> {
            self.active.clone()
        }

        fn unregister_death(&mut self) {
            self.factory = None;
        }
    }

    struct FakeSaves {
        restarts: usize,
        fail: bool,
    }

    impl SavedGameMenuService for FakeSaves {
        fn list(&self) -> SaveListView {
            panic!("unused");
        }

        fn refresh(&mut self) -> Result<(), String> {
            Ok(())
        }

        fn unavailable(&self, _action: SaveAction) -> Option<String> {
            None
        }

        fn save(&mut self, _name: &str, _overwrite: Option<&str>) -> Result<(), String> {
            Ok(())
        }

        fn load(&mut self, _id: &str) -> Result<(), String> {
            Ok(())
        }
    }

    impl PlayerDeathRecovery for FakeSaves {
        fn restart(&mut self) -> Result<(), String> {
            self.restarts += 1;
            if self.fail {
                return Err("restart failed".to_owned());
            }
            Ok(())
        }
    }

    fn death(
        fail: bool,
    ) -> (
        SeatPlayerDeath<FakeController>,
        Rc<RefCell<FakeSaves>>,
        Rc<RefCell<bool>>,
    ) {
        let saves = Rc::new(RefCell::new(FakeSaves { restarts: 0, fail }));
        let quit = Rc::new(RefCell::new(false));
        let quit_inner = Rc::clone(&quit);
        let menu = SeatPlayerDeath::new(
            FakeController {
                factory: None,
                active: None,
                opens: Vec::new(),
                closes: 0,
            },
            Some(saves.clone()),
            Some(saves.clone()),
            UiMenuId::new("menu:saves:load").expect("load menu"),
            Box::new(move || *quit_inner.borrow_mut() = true),
        );
        (menu, saves, quit)
    }

    fn key(code: i32) -> SeatInputEvent {
        SeatInputEvent {
            seat: IdentityOwner::create("death").expect("owner").seat(0),
            time_ms: 0,
            kind: SeatInputEventKind::Key {
                code,
                down: true,
                repeat: false,
            },
        }
    }

    #[test]
    fn death_opens_menu_and_revive_closes() {
        let (mut menu, _, _) = death(false);
        assert!(!menu.active());
        menu.observe(100.0);
        assert!(!menu.active());
        menu.observe(0.0);
        assert!(menu.active());
        assert_eq!(menu.controller.borrow().opens, vec![PLAYER_DEATH_MENU.to_owned()]);
        let factory = menu.controller.borrow().factory.clone().expect("factory");
        let built = factory();
        assert_eq!(built.title, "You died");
        assert!(built.full_screen);
        assert_eq!(built.controls.len(), 3);
        assert!(menu.input(&key(KeyCode::Escape as i32)));
        assert!(!menu.input(&key(65)));
        menu.observe(100.0);
        assert!(!menu.active());
    }

    #[test]
    fn restart_runs_recovery_and_reports_errors() {
        let (menu, saves, _) = death(true);
        menu.inner.borrow_mut().restart();
        assert_eq!(saves.borrow().restarts, 1);
        assert_eq!(menu.inner.borrow().message, "restart failed");
        let factory = menu.controller.borrow().factory.clone().expect("factory");
        let built = factory();
        assert_eq!(built.controls.len(), 4);
        assert_eq!(built.controls[3].label, "restart failed");
    }

    #[test]
    fn without_recovery_nothing_happens() {
        let quit = Rc::new(RefCell::new(false));
        let quit_inner = Rc::clone(&quit);
        let mut menu = SeatPlayerDeath::new(
            FakeController {
                factory: None,
                active: None,
                opens: Vec::new(),
                closes: 0,
            },
            None,
            None,
            UiMenuId::new("menu:saves:load").expect("load menu"),
            Box::new(move || *quit_inner.borrow_mut() = true),
        );
        menu.observe(0.0);
        assert!(!menu.active());
        assert!(!menu.input(&key(KeyCode::Escape as i32)));
        menu.close();
    }
}
