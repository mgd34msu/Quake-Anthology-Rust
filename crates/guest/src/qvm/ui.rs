//! UI module export driver.
//!
//! Provenance: `src/compat/qvm/ui.ts` (Quake III Arena `client/cl_ui.c` and
//! `ui/ui_public.h`). Donor promises become direct returns; [`UiModule`] is
//! a local mirror of the `QvmModule` call surface (owned by another worker).

use super::client_state::AbiProfile;
use super::legacy_bot_abi::{
    UIMENU_BAD_CD_KEY, UIMENU_INGAME, UIMENU_MAIN, UIMENU_NEED_CD, UIMENU_NONE, UIMENU_POSTGAME, UIMENU_TEAM,
    UI_CONSOLE_COMMAND, UI_DRAW_CONNECT_SCREEN, UI_HASUNIQUECDKEY, UI_INIT, UI_IS_FULLSCREEN, UI_KEY_EVENT,
    UI_MOUSE_EVENT, UI_REFRESH, UI_SET_ACTIVE_MENU, UI_SHUTDOWN,
};
use crate::error::GuestError;

/// UI menu command.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiMenu {
    /// No menu.
    None,
    /// Main menu.
    Main,
    /// In-game menu.
    Ingame,
    /// Need-CD menu.
    NeedCd,
    /// Bad-CD-key menu.
    BadCdKey,
    /// Team menu.
    Team,
    /// Postgame menu.
    Postgame,
}

/// Menu command to export number.
#[must_use]
pub const fn menu_number(menu: UiMenu) -> i32 {
    match menu {
        UiMenu::None => UIMENU_NONE,
        UiMenu::Main => UIMENU_MAIN,
        UiMenu::Ingame => UIMENU_INGAME,
        UiMenu::NeedCd => UIMENU_NEED_CD,
        UiMenu::BadCdKey => UIMENU_BAD_CD_KEY,
        UiMenu::Team => UIMENU_TEAM,
        UiMenu::Postgame => UIMENU_POSTGAME,
    }
}

/// Guest UI module call surface.
pub trait UiModule {
    /// ABI profile of the module.
    fn abi_profile(&self) -> AbiProfile;
    /// Invoke an export with trap words.
    fn call(&mut self, words: &[i32]) -> Result<i32, GuestError>;
    /// Invoke a console-command export with argv.
    fn command(&mut self, words: &[i32], args: &[String]) -> Result<i32, GuestError>;
    /// Retire the module.
    fn retire(&mut self);
}

/// Owned UI module driver for one seat.
pub struct QvmUi<M> {
    /// Seat identifier.
    pub seat: u32,
    module: M,
    assert_current: Box<dyn Fn() -> Result<(), GuestError>>,
    retired: bool,
}

impl<M: UiModule> QvmUi<M> {
    /// Wrap a UI module for `seat`.
    pub fn new(
        seat: u32,
        module: M,
        assert_current: impl Fn() -> Result<(), GuestError> + 'static,
    ) -> Result<Self, GuestError> {
        assert_current()?;
        Ok(Self {
            seat,
            module,
            assert_current: Box::new(assert_current),
            retired: false,
        })
    }

    fn current(&self) -> Result<(), GuestError> {
        (self.assert_current)()?;
        if self.retired {
            return Err(GuestError::runtime("UI module has been retired"));
        }
        Ok(())
    }

    fn invoke(&mut self, words: &[i32]) -> Result<i32, GuestError> {
        self.current()?;
        let result = self.module.call(words)?;
        self.current()?;
        Ok(result)
    }

    /// Initialize the UI.
    pub fn init(&mut self, connecting: bool) -> Result<(), GuestError> {
        self.invoke(&[UI_INIT, i32::from(connecting)])?;
        Ok(())
    }

    /// Shut down the UI.
    pub fn shutdown(&mut self) -> Result<(), GuestError> {
        self.invoke(&[UI_SHUTDOWN])?;
        Ok(())
    }

    /// Retire the module.
    pub fn retire(&mut self) {
        self.retired = true;
        self.module.retire();
    }

    /// Deliver a key event.
    pub fn key_event(&mut self, key: i32, down: bool) -> Result<(), GuestError> {
        self.invoke(&[UI_KEY_EVENT, key, i32::from(down)])?;
        Ok(())
    }

    /// Deliver a mouse event.
    pub fn mouse_event(&mut self, dx: i32, dy: i32) -> Result<(), GuestError> {
        self.invoke(&[UI_MOUSE_EVENT, dx, dy])?;
        Ok(())
    }

    /// Refresh the UI.
    pub fn refresh(&mut self, time: i32) -> Result<(), GuestError> {
        self.invoke(&[UI_REFRESH, time])?;
        Ok(())
    }

    /// Query fullscreen state.
    pub fn is_fullscreen(&mut self) -> Result<bool, GuestError> {
        Ok(self.invoke(&[UI_IS_FULLSCREEN])? != 0)
    }

    /// Set the active menu.
    pub fn set_active_menu(&mut self, menu: UiMenu) -> Result<(), GuestError> {
        if self.module.abi_profile() != AbiProfile::Modern && menu_number(menu) > UIMENU_BAD_CD_KEY {
            return Err(GuestError::invalid(format!(
                "Legacy UI does not implement menu {menu:?}"
            )));
        }
        self.invoke(&[UI_SET_ACTIVE_MENU, menu_number(menu)])?;
        Ok(())
    }

    /// Dispatch a console command.
    pub fn console_command(&mut self, time: i32, args: &[String]) -> Result<bool, GuestError> {
        self.current()?;
        let result = self.module.command(&[UI_CONSOLE_COMMAND, time], args)? != 0;
        self.current()?;
        Ok(result)
    }

    /// Draw the connect screen.
    pub fn draw_connect_screen(&mut self, overlay: bool) -> Result<(), GuestError> {
        self.invoke(&[UI_DRAW_CONNECT_SCREEN, i32::from(overlay)])?;
        Ok(())
    }

    /// Query unique-CD-key support (modern ABI only).
    pub fn has_unique_cd_key(&mut self) -> Result<bool, GuestError> {
        if self.module.abi_profile() != AbiProfile::Modern {
            return Err(GuestError::invalid("Legacy UI does not export UI_HASUNIQUECDKEY"));
        }
        Ok(self.invoke(&[UI_HASUNIQUECDKEY])? != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeModule {
        profile: AbiProfile,
        calls: Vec<Vec<i32>>,
        commands: Vec<(Vec<i32>, Vec<String>)>,
        retired: bool,
        result: i32,
    }

    impl FakeModule {
        fn new(profile: AbiProfile) -> Self {
            Self {
                profile,
                calls: Vec::new(),
                commands: Vec::new(),
                retired: false,
                result: 0,
            }
        }
    }

    impl UiModule for FakeModule {
        fn abi_profile(&self) -> AbiProfile {
            self.profile
        }
        fn call(&mut self, words: &[i32]) -> Result<i32, GuestError> {
            self.calls.push(words.to_vec());
            Ok(self.result)
        }
        fn command(&mut self, words: &[i32], args: &[String]) -> Result<i32, GuestError> {
            self.commands.push((words.to_vec(), args.to_vec()));
            Ok(self.result)
        }
        fn retire(&mut self) {
            self.retired = true;
        }
    }

    fn ui(profile: AbiProfile) -> QvmUi<FakeModule> {
        QvmUi::new(1, FakeModule::new(profile), || Ok(())).unwrap()
    }

    #[test]
    fn lifecycle_calls() {
        let mut ui = ui(AbiProfile::Modern);
        ui.init(true).unwrap();
        ui.key_event(65, true).unwrap();
        ui.mouse_event(3, -2).unwrap();
        ui.refresh(100).unwrap();
        ui.shutdown().unwrap();
        let calls = &ui.module.calls;
        assert_eq!(
            calls,
            &vec![
                vec![UI_INIT, 1],
                vec![UI_KEY_EVENT, 65, 1],
                vec![UI_MOUSE_EVENT, 3, -2],
                vec![UI_REFRESH, 100],
                vec![UI_SHUTDOWN],
            ]
        );
    }

    #[test]
    fn fullscreen_and_menus() {
        let mut ui = ui(AbiProfile::Modern);
        ui.module.result = 1;
        assert!(ui.is_fullscreen().unwrap());
        ui.set_active_menu(UiMenu::Postgame).unwrap();
        ui.draw_connect_screen(false).unwrap();
        assert_eq!(ui.module.calls[1], vec![UI_SET_ACTIVE_MENU, 6]);
        assert!(ui.console_command(9, &["quit".to_string()]).unwrap());
        assert_eq!(ui.module.commands[0].0, vec![UI_CONSOLE_COMMAND, 9]);
    }

    #[test]
    fn menu_numbers() {
        assert_eq!(menu_number(UiMenu::None), 0);
        assert_eq!(menu_number(UiMenu::Main), 1);
        assert_eq!(menu_number(UiMenu::Ingame), 2);
        assert_eq!(menu_number(UiMenu::NeedCd), 3);
        assert_eq!(menu_number(UiMenu::BadCdKey), 4);
        assert_eq!(menu_number(UiMenu::Team), 5);
        assert_eq!(menu_number(UiMenu::Postgame), 6);
    }

    #[test]
    fn legacy_restrictions() {
        let mut ui = ui(AbiProfile::Legacy);
        ui.set_active_menu(UiMenu::BadCdKey).unwrap();
        assert!(ui.set_active_menu(UiMenu::Team).is_err());
        assert!(ui.has_unique_cd_key().is_err());
    }

    #[test]
    fn retire_blocks_calls() {
        let mut ui = ui(AbiProfile::Modern);
        ui.retire();
        assert!(ui.module.retired);
        assert!(ui.refresh(0).is_err());
    }
}
