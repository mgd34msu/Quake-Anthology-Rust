//! Mod menus.
//!
//! Ported from donor `src/ui/mods/*`.

pub mod menu;

pub use menu::{
    build_mod_menu, register_mod_menu, ModMenuInputs, ModMenuRow, ModMenuService, ModMenuState, ModMenus, MODS_MENU_ID,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_menu_id_resolves() {
        assert_eq!(MODS_MENU_ID, "menu:mods:root");
    }
}
