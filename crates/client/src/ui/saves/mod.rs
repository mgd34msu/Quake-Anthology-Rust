//! Saved-game menus.

pub mod menu;

pub use menu::{
    register_saved_game_menus, SaveAction, SaveListRow, SaveListView, SavedGameMenuService, SavedGameMenus,
    LOAD_MENU_ID, SAVE_CONFIRM_MENU_ID, SAVE_MENU_ID, SAVE_NAME_MENU_ID,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_menu_ids_and_empty_list_resolve() {
        assert_eq!(SAVE_MENU_ID, "menu:saves:save");
        assert_eq!(LOAD_MENU_ID, "menu:saves:load");
        assert_eq!(SAVE_NAME_MENU_ID, "menu:saves:name");
        assert_eq!(SAVE_CONFIRM_MENU_ID, "menu:saves:overwrite");
        assert!(SaveListView::empty().rows.is_empty());
    }
}
