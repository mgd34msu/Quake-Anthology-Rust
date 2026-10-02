//! Single-player arena menus: selection, difficulty, progress, and results.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/base-arena-menu.ts`
//! (`BaseArenaMenus`, `BaseArenaMenuService`). Services and the controller are
//! shared trait objects because menu factories are `'static`; the result
//! identity uses debug text instead of JSON.

use std::cell::RefCell;
use std::rc::Rc;

use qa_client::text::draw2d::Rect;
use qa_client::ui::types::{UiControl, UiControlId, UiControlKind, UiListRow, UiMenu, UiMenuId};

use super::base_arena_postgame::ArenaPostgamePresentation;
use super::base_arena_progression::ArenaSkill;
use super::base_arena_select_menu::{
    close_arena_menu, open_arena_menu, register_arena_selection_menu, ArenaMenuController, ArenaSelectionMenuService,
};
use super::base_arena_selection::ArenaSelection;

/// One progress row (`progress` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArenaProgressRow {
    /// Row label.
    pub label: String,
    /// Row value.
    pub value: String,
}

/// Arena menu service (`BaseArenaMenuService`).
pub trait BaseArenaMenuService {
    /// Read the arena selection.
    fn selection(&self) -> ArenaSelection;
    /// Read the current skill.
    fn skill(&self) -> ArenaSkill;
    /// Play a map at a skill.
    fn play(&mut self, map: &str, skill: ArenaSkill);
    /// Read the latest postgame presentation, if a match ended.
    fn result(&self) -> Option<ArenaPostgamePresentation>;
    /// Player name for a client number.
    fn player_name(&self, client: i32) -> String;
    /// Progress rows.
    fn progress(&self) -> Vec<ArenaProgressRow>;
    /// Retry the match.
    fn retry(&mut self);
    /// Advance to the next match.
    fn next(&mut self);
    /// Quit to the main menu.
    fn quit(&mut self);
    /// Reset progress.
    fn reset(&mut self);
}

fn menu_id(id: &str) -> UiMenuId {
    UiMenuId::new(id).expect("static arena menu id")
}

fn control_id(id: &str) -> UiControlId {
    UiControlId::new(id).expect("static arena control id")
}

fn arena_button(id: &str, label: String, y: f32, enabled: bool, activate: impl Fn() + 'static) -> UiControl {
    UiControl {
        id: control_id(&format!("ui:arena:{id}")),
        label,
        rect: Rect {
            x: 64.0,
            y,
            width: 512.0,
            height: 32.0,
        },
        enabled,
        visible: true,
        kind: UiControlKind::Button {
            on_activate: Rc::new(move |_| activate()),
        },
    }
}

struct SelectionMenuAdapter {
    service: Rc<RefCell<dyn BaseArenaMenuService>>,
    controller: Rc<RefCell<dyn ArenaMenuController>>,
    selected_map: Rc<RefCell<Option<String>>>,
    difficulty_menu: UiMenuId,
}

impl ArenaSelectionMenuService for SelectionMenuAdapter {
    fn read(&self) -> Option<ArenaSelection> {
        Some(self.service.borrow().selection())
    }

    fn choose(&mut self, map: &str) {
        *self.selected_map.borrow_mut() = Some(map.to_string());
        open_arena_menu(&self.controller, &self.difficulty_menu);
    }
}

/// Arena menus (`BaseArenaMenus`).
pub struct BaseArenaMenus {
    controller: Rc<RefCell<dyn ArenaMenuController>>,
    service: Rc<RefCell<dyn BaseArenaMenuService>>,
    root: UiMenuId,
    shown: Option<String>,
    disposers: Vec<Box<dyn FnOnce()>>,
}

impl BaseArenaMenus {
    /// Register the arena menus.
    pub fn new(
        controller: Rc<RefCell<dyn ArenaMenuController>>,
        service: Rc<RefCell<dyn BaseArenaMenuService>>,
    ) -> Self {
        let root = menu_id("menu:application:arena-progress");
        let selection_menu = menu_id("menu:application:arena-selection");
        let difficulty_menu = menu_id("menu:application:arena-skill");
        let reset_menu = menu_id("menu:application:arena-reset");
        let result_menu = menu_id("menu:application:arena-result");
        let selected_map: Rc<RefCell<Option<String>>> = Rc::new(RefCell::new(None));

        let adapter = Rc::new(RefCell::new(SelectionMenuAdapter {
            service: Rc::clone(&service),
            controller: Rc::clone(&controller),
            selected_map: Rc::clone(&selected_map),
            difficulty_menu: difficulty_menu.clone(),
        }));
        let mut disposers: Vec<Box<dyn FnOnce()>> = vec![register_arena_selection_menu(
            Rc::clone(&controller),
            selection_menu.clone(),
            adapter,
        )];

        let difficulties = [
            (ArenaSkill::One, "I Can Win"),
            (ArenaSkill::Two, "Bring It On"),
            (ArenaSkill::Three, "Hurt Me Plenty"),
            (ArenaSkill::Four, "Hardcore"),
            (ArenaSkill::Five, "Nightmare"),
        ];
        {
            let service = Rc::clone(&service);
            let controller = Rc::clone(&controller);
            let selected_map = Rc::clone(&selected_map);
            let difficulty_menu = difficulty_menu.clone();
            let unregister_controller = Rc::clone(&controller);
            let unregister_id = difficulty_menu.clone();
            let registrar = Rc::clone(&controller);
            let factory = Rc::new(move || {
                let current_skill = service.borrow().skill();
                let mut controls: Vec<UiControl> = difficulties
                    .iter()
                    .enumerate()
                    .map(|(index, (skill, label))| {
                        let marker = if *skill == current_skill { "> " } else { "" };
                        let service = Rc::clone(&service);
                        let selected_map = Rc::clone(&selected_map);
                        let skill = *skill;
                        arena_button(
                            &format!("skill:{}", skill.value()),
                            format!("{marker}{label}"),
                            108.0 + index as f32 * 44.0,
                            true,
                            move || {
                                if let Some(map) = selected_map.borrow().clone() {
                                    service.borrow_mut().play(&map, skill);
                                }
                            },
                        )
                    })
                    .collect();
                let back_controller = Rc::clone(&controller);
                controls.push(arena_button("skill-back", "Back".to_string(), 420.0, true, move || {
                    close_arena_menu(&back_controller);
                }));
                UiMenu {
                    scroll: None,
                    id: difficulty_menu.clone(),
                    title: "Difficulty".to_string(),
                    full_screen: true,
                    controls,
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            });
            registrar.borrow_mut().register_menu(unregister_id.clone(), factory);
            let unregister = {
                Box::new(move || unregister_controller.borrow_mut().unregister_menu(&unregister_id))
                    as Box<dyn FnOnce()>
            };
            disposers.push(unregister);
        }

        {
            let service = Rc::clone(&service);
            let controller = Rc::clone(&controller);
            let root = root.clone();
            let selection_menu = selection_menu.clone();
            let reset_menu = reset_menu.clone();
            let unregister_controller = Rc::clone(&controller);
            let unregister_id = root.clone();
            let registrar = Rc::clone(&controller);
            let factory = Rc::new(move || {
                let rows = service
                    .borrow()
                    .progress()
                    .into_iter()
                    .enumerate()
                    .map(|(index, row)| UiListRow {
                        action: None,
                        id: index.to_string(),
                        cells: vec![row.label, row.value],
                        image: None,
                        enabled: true,
                    })
                    .collect();
                let select_controller = Rc::clone(&controller);
                let select_menu = selection_menu.clone();
                let reset_controller = Rc::clone(&controller);
                let reset_menu = reset_menu.clone();
                let back_controller = Rc::clone(&controller);
                UiMenu {
                    scroll: None,
                    id: root.clone(),
                    title: "Arena progress".to_string(),
                    full_screen: true,
                    controls: vec![
                        UiControl {
                            id: control_id("ui:arena:progress"),
                            label: "Progress and awards".to_string(),
                            rect: Rect {
                                x: 48.0,
                                y: 96.0,
                                width: 544.0,
                                height: 231.0,
                            },
                            enabled: true,
                            visible: true,
                            kind: UiControlKind::List {
                                row_height: Some(33.0),
                                column_widths: None,
                                on_activate: None,
                                rows,
                                selected: None,
                                on_select: Rc::new(|_, _| {}),
                            },
                        },
                        arena_button("select", "Choose an arena".to_string(), 336.0, true, move || {
                            open_arena_menu(&select_controller, &select_menu);
                        }),
                        arena_button("reset", "Reset progress...".to_string(), 376.0, true, move || {
                            open_arena_menu(&reset_controller, &reset_menu);
                        }),
                        arena_button("back", "Back".to_string(), 424.0, true, move || {
                            close_arena_menu(&back_controller);
                        }),
                    ],
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            });
            registrar.borrow_mut().register_menu(unregister_id.clone(), factory);
            let unregister = {
                Box::new(move || unregister_controller.borrow_mut().unregister_menu(&unregister_id))
                    as Box<dyn FnOnce()>
            };
            disposers.push(unregister);
        }

        {
            let service = Rc::clone(&service);
            let controller = Rc::clone(&controller);
            let reset_menu = reset_menu.clone();
            let unregister_controller = Rc::clone(&controller);
            let unregister_id = reset_menu.clone();
            let registrar = Rc::clone(&controller);
            let factory = Rc::new(move || {
                let cancel_controller = Rc::clone(&controller);
                let confirm_service = Rc::clone(&service);
                let confirm_controller = Rc::clone(&controller);
                UiMenu {
                    scroll: None,
                    id: reset_menu.clone(),
                    title: "Reset arena progress and awards?".to_string(),
                    full_screen: true,
                    controls: vec![
                        arena_button("cancel", "Keep progress".to_string(), 180.0, true, move || {
                            close_arena_menu(&cancel_controller);
                        }),
                        arena_button("confirm-reset", "Reset progress".to_string(), 230.0, true, move || {
                            confirm_service.borrow_mut().reset();
                            close_arena_menu(&confirm_controller);
                        }),
                    ],
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            });
            registrar.borrow_mut().register_menu(unregister_id.clone(), factory);
            let unregister = {
                Box::new(move || unregister_controller.borrow_mut().unregister_menu(&unregister_id))
                    as Box<dyn FnOnce()>
            };
            disposers.push(unregister);
        }

        {
            let service = Rc::clone(&service);
            let controller = Rc::clone(&controller);
            let result_menu = result_menu.clone();
            let selection_menu = selection_menu.clone();
            let root = root.clone();
            let unregister_controller = Rc::clone(&controller);
            let unregister_id = result_menu.clone();
            let registrar = Rc::clone(&controller);
            let factory = Rc::new(move || {
                let result = service.borrow().result();
                let mut controls: Vec<UiControl> = result
                    .as_ref()
                    .map(|result| result.podium.clone())
                    .unwrap_or_default()
                    .into_iter()
                    .enumerate()
                    .map(|(index, player)| {
                        let name = service.borrow().player_name(player.client);
                        arena_button(
                            &format!("podium:{index}"),
                            format!("{}. {name}   {}", player.rank, player.score),
                            96.0 + index as f32 * 38.0,
                            false,
                            || {},
                        )
                    })
                    .collect();
                let retry_service = Rc::clone(&service);
                controls.push(arena_button("retry", "Retry".to_string(), 258.0, true, move || {
                    retry_service.borrow_mut().retry();
                }));
                let next_service = Rc::clone(&service);
                let next_enabled = result.as_ref().is_some_and(|result| result.result.next_level >= 0);
                controls.push(arena_button(
                    "next",
                    "Next match".to_string(),
                    300.0,
                    next_enabled,
                    move || {
                        next_service.borrow_mut().next();
                    },
                ));
                let arenas_controller = Rc::clone(&controller);
                let arenas_menu = selection_menu.clone();
                controls.push(arena_button(
                    "arenas",
                    "Choose an arena".to_string(),
                    342.0,
                    true,
                    move || {
                        open_arena_menu(&arenas_controller, &arenas_menu);
                    },
                ));
                let awards_controller = Rc::clone(&controller);
                let awards_menu = root.clone();
                controls.push(arena_button(
                    "awards",
                    "Progress and awards".to_string(),
                    384.0,
                    true,
                    move || {
                        open_arena_menu(&awards_controller, &awards_menu);
                    },
                ));
                let main_service = Rc::clone(&service);
                controls.push(arena_button("main", "Main menu".to_string(), 426.0, true, move || {
                    main_service.borrow_mut().quit();
                }));
                let victory = result.as_ref().is_some_and(|result| result.result.rank == 1);
                UiMenu {
                    scroll: None,
                    id: result_menu.clone(),
                    title: if victory {
                        "Victory".to_string()
                    } else {
                        "Match complete".to_string()
                    },
                    full_screen: true,
                    controls,
                    on_open: Rc::new(|_| {}),
                    on_close: Rc::new(|_| {}),
                }
            });
            registrar.borrow_mut().register_menu(unregister_id.clone(), factory);
            let unregister = {
                Box::new(move || unregister_controller.borrow_mut().unregister_menu(&unregister_id))
                    as Box<dyn FnOnce()>
            };
            disposers.push(unregister);
        }

        Self {
            controller,
            service,
            root,
            shown: None,
            disposers,
        }
    }

    /// Progress menu id.
    #[must_use]
    pub fn root(&self) -> &UiMenuId {
        &self.root
    }

    /// Show the result menu when a new match ended (`update`).
    pub fn update(&mut self) {
        let result = self.service.borrow().result();
        let Some(result) = result else {
            self.shown = None;
            return;
        };
        let identity = format!("{:?}|{:?}", result.result, result.podium);
        if self.shown.as_deref() == Some(&identity) {
            return;
        }
        self.shown = Some(identity);
        self.controller.borrow_mut().close_all_menus();
        open_arena_menu(&self.controller, &menu_id("menu:application:arena-result"));
    }

    /// Unregister the arena menus (`close`).
    pub fn close(self) {
        for dispose in self.disposers {
            dispose();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::base_arena_catalog::BaseArena;
    use super::super::base_arena_postgame::{ArenaPodiumPlayer, PostgameControl, PostgameMusic};
    use super::super::base_arena_progression::ArenaProgressionResult;
    use super::super::base_arena_selection::{ArenaSelection, ArenaSelectionRow, ArenaSelectionTier};
    use super::*;
    use qa_core::identity::{IdentityOwner, SeatId};

    struct StubController {
        menus: std::collections::HashMap<UiMenuId, qa_client::ui::common::controller::UiMenuFactory>,
        opened: Vec<UiMenuId>,
        cleared: usize,
    }

    impl StubController {
        fn build(&self, id: &UiMenuId) -> UiMenu {
            self.menus.get(id).unwrap()()
        }
    }

    impl ArenaMenuController for StubController {
        fn register_menu(&mut self, id: UiMenuId, factory: qa_client::ui::common::controller::UiMenuFactory) {
            self.menus.insert(id, factory);
        }

        fn unregister_menu(&mut self, id: &UiMenuId) {
            self.menus.remove(id);
        }

        fn open_menu(&mut self, id: &UiMenuId) -> Result<(), qa_client::ClientError> {
            self.opened.push(id.clone());
            Ok(())
        }

        fn close_menu(&mut self) {}

        fn close_all_menus(&mut self) {
            self.cleared += 1;
        }
    }

    struct StubService {
        selection: ArenaSelection,
        skill: ArenaSkill,
        played: Vec<(String, ArenaSkill)>,
        result: Option<ArenaPostgamePresentation>,
        progress: Vec<ArenaProgressRow>,
        calls: Vec<String>,
    }

    impl BaseArenaMenuService for StubService {
        fn selection(&self) -> ArenaSelection {
            self.selection.clone()
        }

        fn skill(&self) -> ArenaSkill {
            self.skill
        }

        fn play(&mut self, map: &str, skill: ArenaSkill) {
            self.played.push((map.to_string(), skill));
        }

        fn result(&self) -> Option<ArenaPostgamePresentation> {
            self.result.clone()
        }

        fn player_name(&self, client: i32) -> String {
            format!("Player{client}")
        }

        fn progress(&self) -> Vec<ArenaProgressRow> {
            self.progress.clone()
        }

        fn retry(&mut self) {
            self.calls.push("retry".to_string());
        }

        fn next(&mut self) {
            self.calls.push("next".to_string());
        }

        fn quit(&mut self) {
            self.calls.push("quit".to_string());
        }

        fn reset(&mut self) {
            self.calls.push("reset".to_string());
        }
    }

    fn test_selection() -> ArenaSelection {
        ArenaSelection {
            tiers: vec![ArenaSelectionTier {
                id: "1".to_string(),
                label: "Tier 1".to_string(),
            }],
            rows: vec![ArenaSelectionRow {
                arena: BaseArena {
                    number: 1,
                    map: "maps/q3dm1.bsp".to_string(),
                    title: "Arena".to_string(),
                    bots: vec!["Sarge".to_string()],
                    special: String::new(),
                    selection: 1,
                    frag_limit: 10,
                    time_limit: 0,
                },
                tier: "1".to_string(),
                available: true,
                record: "Not completed".to_string(),
            }],
            current: Some("maps/q3dm1.bsp".to_string()),
        }
    }

    fn test_result(next_level: i32) -> ArenaPostgamePresentation {
        ArenaPostgamePresentation {
            result: ArenaProgressionResult {
                rank: 1,
                completed_tier: 1,
                unlocked_movie: None,
                awards: Vec::new(),
                next_level,
            },
            podium: vec![ArenaPodiumPlayer {
                client: 0,
                rank: 1,
                score: 10,
            }],
            music: PostgameMusic::Win,
            winner_announcement_after_ms: 0,
            controls: [PostgameControl::Retry, PostgameControl::Next, PostgameControl::Main],
        }
    }

    fn seat() -> SeatId {
        IdentityOwner::create("test").unwrap().seat(0)
    }

    fn activate(menu: &UiMenu, index: usize) {
        match &menu.controls[index].kind {
            UiControlKind::Button { on_activate } => on_activate(seat()),
            kind => panic!("expected button, got {kind:?}"),
        }
    }

    fn harness() -> (Rc<RefCell<StubController>>, Rc<RefCell<StubService>>, BaseArenaMenus) {
        let stub = Rc::new(RefCell::new(StubController {
            menus: std::collections::HashMap::new(),
            opened: Vec::new(),
            cleared: 0,
        }));
        let controller: Rc<RefCell<dyn ArenaMenuController>> = stub.clone();
        let service = Rc::new(RefCell::new(StubService {
            selection: test_selection(),
            skill: ArenaSkill::Three,
            played: Vec::new(),
            result: None,
            progress: vec![ArenaProgressRow {
                label: "Wins".to_string(),
                value: "3".to_string(),
            }],
            calls: Vec::new(),
        }));
        let menus = BaseArenaMenus::new(controller, Rc::clone(&service) as Rc<RefCell<dyn BaseArenaMenuService>>);
        (stub, service, menus)
    }

    #[test]
    fn registers_all_menus() {
        let (stub, _service, menus) = harness();
        assert_eq!(stub.borrow().menus.len(), 5);
        assert_eq!(menus.root(), &menu_id("menu:application:arena-progress"));
        let root = stub.borrow().build(&menu_id("menu:application:arena-progress"));
        assert_eq!(root.title, "Arena progress");
        match &root.controls[0].kind {
            UiControlKind::List { rows, .. } => {
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].cells, vec!["Wins".to_string(), "3".to_string()]);
            }
            kind => panic!("expected list, got {kind:?}"),
        }
        menus.close();
        assert!(stub.borrow().menus.is_empty());
    }

    #[test]
    fn difficulty_marks_skill_and_plays_choice() {
        let (stub, service, _menus) = harness();
        let selection = stub.borrow().build(&menu_id("menu:application:arena-selection"));
        match &selection.controls[4].kind {
            UiControlKind::Button { on_activate } => on_activate(seat()),
            kind => panic!("expected button, got {kind:?}"),
        }
        assert_eq!(stub.borrow().opened.len(), 1);
        let difficulty = stub.borrow().build(&menu_id("menu:application:arena-skill"));
        assert!(difficulty.controls[2].label.starts_with("> "));
        assert_eq!(difficulty.controls[2].label, "> Hurt Me Plenty");
        activate(&difficulty, 4);
        assert_eq!(
            service.borrow().played,
            vec![("maps/q3dm1.bsp".to_string(), ArenaSkill::Five)]
        );
    }

    #[test]
    fn update_shows_new_results_once() {
        let (stub, service, mut menus) = harness();
        menus.update();
        assert!(stub.borrow().opened.is_empty());
        service.borrow_mut().result = Some(test_result(2));
        menus.update();
        menus.update();
        assert_eq!(stub.borrow().opened.len(), 1);
        assert_eq!(stub.borrow().cleared, 1);
        let result = stub.borrow().build(&menu_id("menu:application:arena-result"));
        assert_eq!(result.title, "Victory");
        assert!(result.controls[1].enabled);
        activate(&result, 1);
        assert_eq!(service.borrow().calls, vec!["retry".to_string()]);
        service.borrow_mut().result = Some(test_result(-1));
        menus.update();
        let result = stub.borrow().build(&menu_id("menu:application:arena-result"));
        assert!(!result.controls[2].enabled);
    }

    #[test]
    fn reset_confirm_resets_and_progress_lists() {
        let (stub, service, _menus) = harness();
        let reset = stub.borrow().build(&menu_id("menu:application:arena-reset"));
        activate(&reset, 1);
        assert_eq!(service.borrow().calls, vec!["reset".to_string()]);
    }
}
