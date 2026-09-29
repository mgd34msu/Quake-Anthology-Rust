//! Legacy Q3 menu support.

pub mod borders;
pub mod input;
pub mod menu;
pub mod runtime;
pub mod script;
pub mod team_arena;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn submodule_surfaces_resolve() {
        let _ = borders::draw_cg_rect
            as fn(
                &mut crate::text::draw2d::Draw2D,
                &crate::text::draw2d::Rect,
                f32,
                qa_core::math::Vec4,
                crate::text::draw2d::PictureAsset,
            );
        assert_eq!(menu::MAX_UI_MENUS, 64);
        assert_eq!(menu::MAX_UI_MENU_ITEMS, 96);
        assert_eq!(script::DEFAULT_SCRIPT_TOKEN_LIMIT, 1024);
        fn assert_resolves<T>(_: &str) {}
        assert_resolves::<input::LegacyUiSeat>("seat");
        assert_resolves::<runtime::UiRuntime>("runtime");
        assert_resolves::<team_arena::memory::UiStringReference>("reference");
    }
}
