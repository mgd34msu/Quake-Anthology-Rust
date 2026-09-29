//! Library menus: inventory, match controls, and the generic menu.
//!
//! Ported from donor `src/ui/library/*`.

pub mod inventory;
pub mod match_menu;
pub mod menu;

pub use inventory::{
    build_inventory_menu, register_inventory_menu, InventoryMenuInputs, InventoryMenus, INVENTORY_MENU_ID,
};
pub use match_menu::{
    build_match_menu, register_match_menu, MatchMenuInputs, MatchMenuState, MatchMenus, MATCH_MENU_ID,
};
pub use menu::{
    build_library_menu, register_library_menu, LibraryEntry, LibraryMenuInputs, LibraryMenuService, LibraryMenuState,
    LibraryMenus,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reexported_menu_ids_resolve() {
        assert_eq!(INVENTORY_MENU_ID, "menu:application:inventory");
        assert_eq!(MATCH_MENU_ID, "menu:application:match");
    }
}
