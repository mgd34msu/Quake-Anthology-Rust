//! Match controls menu.
//!
//! Ported from donor `src/ui/library/match.ts`: team join, player follow,
//! voting, and bot management. Commands travel through the seat's existing
//! source dispatcher and permission checks.

use std::cell::RefCell;
use std::rc::Rc;

use crate::text::draw2d::Rect;
use crate::ui::common::controller::NativeUiController;
use crate::ui::types::{
    SeatUiController as _, UiChoice, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId, UiMenuScroll,
};

/// Match menu root (`menu:application:match`).
pub const MATCH_MENU_ID: &str = "menu:application:match";

/// Command dispatcher `(name, args)`.
pub type CommandFn = Rc<dyn Fn(&str, &[String])>;

fn menu_id(text: &str) -> UiMenuId {
    match UiMenuId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI menu id is invalid: {text}"),
    }
}

fn control_id(text: &str) -> UiControlId {
    match UiControlId::new(text) {
        Ok(id) => id,
        Err(_) => panic!("static UI control id is invalid: {text}"),
    }
}

/// Mutable match menu field state.
#[derive(Debug, Clone)]
pub struct MatchMenuState {
    /// Player-or-map text field.
    pub target: String,
    /// Selected team id.
    pub team: String,
    /// Selected vote id.
    pub vote: String,
    /// Bot name text field.
    pub bot: String,
    /// Selected bot skill id.
    pub skill: String,
}

impl Default for MatchMenuState {
    fn default() -> Self {
        Self {
            target: String::new(),
            team: "red".to_string(),
            vote: "map_restart".to_string(),
            bot: "sarge".to_string(),
            skill: "3".to_string(),
        }
    }
}

/// Live match menu inputs.
pub struct MatchMenuInputs {
    /// Command dispatcher `(name, args)`.
    pub command: CommandFn,
    /// Shared controller for the Back button.
    pub controller: Rc<RefCell<NativeUiController>>,
    /// Mutable field state.
    pub state: Rc<RefCell<MatchMenuState>>,
}

/// Registered match menu: root id plus disposal.
pub struct MatchMenus {
    /// Menu root id.
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
}

impl MatchMenus {
    /// Unregister the match menu.
    pub fn dispose(self) {
        self.controller.borrow_mut().unregister(&self.root);
    }
}

impl std::fmt::Debug for MatchMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MatchMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

fn match_button(
    suffix: &str,
    label: &str,
    row: i32,
    enabled: bool,
    on_activate: Rc<dyn Fn(qa_core::identity::SeatId)>,
) -> UiControl {
    UiControl {
        id: control_id(&format!("ui:match:{suffix}")),
        label: label.to_string(),
        rect: Rect {
            x: 64.0,
            y: 96.0 + (row as f32) * 34.0,
            width: 496.0,
            height: 30.0,
        },
        enabled,
        visible: true,
        kind: UiControlKind::Button { on_activate },
    }
}

fn command_button(
    state: &Rc<RefCell<MatchMenuState>>,
    command: &CommandFn,
    suffix: &str,
    label: &str,
    row: i32,
    enabled: bool,
    build: impl Fn(&MatchMenuState) -> (String, Vec<String>) + 'static,
) -> UiControl {
    let state = Rc::clone(state);
    let command = Rc::clone(command);
    match_button(
        suffix,
        label,
        row,
        enabled,
        Rc::new(move |_| {
            let (name, args) = build(&state.borrow());
            command(&name, &args);
        }),
    )
}

/// Build one match menu frame from the current field state.
pub fn build_match_menu(root: &UiMenuId, inputs: &MatchMenuInputs) -> UiMenu {
    let snapshot = inputs.state.borrow().clone();
    let state = Rc::clone(&inputs.state);
    let command = Rc::clone(&inputs.command);
    let team_state = Rc::clone(&state);
    let target_state = Rc::clone(&state);
    let target_submit = Rc::clone(&state);
    let vote_state = Rc::clone(&state);
    let bot_state = Rc::clone(&state);
    let bot_submit = Rc::clone(&state);
    let skill_state = Rc::clone(&state);
    let back_controller = Rc::clone(&inputs.controller);
    let follow_enabled = !snapshot.target.trim().is_empty();
    let vote_needs_target = snapshot.vote == "map" || snapshot.vote == "kick";
    let callvote_enabled = !vote_needs_target || !snapshot.target.trim().is_empty();
    let bot_enabled = !snapshot.bot.trim().is_empty();
    let target_text = snapshot.target.clone();
    let bot_text = snapshot.bot.clone();
    let team_selected = snapshot.team.clone();
    let vote_selected = snapshot.vote.clone();
    let skill_selected = snapshot.skill.clone();
    let controls = vec![
        UiControl {
            id: control_id("ui:match:team"),
            label: "Team".to_string(),
            rect: Rect {
                x: 64.0,
                y: 96.0,
                width: 496.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Choice {
                choices: vec![
                    UiChoice {
                        id: "red".to_string(),
                        label: "Red".to_string(),
                    },
                    UiChoice {
                        id: "blue".to_string(),
                        label: "Blue".to_string(),
                    },
                    UiChoice {
                        id: "free".to_string(),
                        label: "Free for all".to_string(),
                    },
                    UiChoice {
                        id: "spectator".to_string(),
                        label: "Spectator".to_string(),
                    },
                ],
                selected: Some(team_selected),
                on_select: Rc::new(move |_, value: &str| {
                    team_state.borrow_mut().team = value.to_string();
                }),
            },
        },
        command_button(&state, &command, "join", "Join team", 1, true, |state| {
            ("team".to_string(), vec![state.team.clone()])
        }),
        UiControl {
            id: control_id("ui:match:target"),
            label: "Player or map".to_string(),
            rect: Rect {
                x: 64.0,
                y: 164.0,
                width: 496.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: target_text,
                maximum_length: 64,
                on_change: Rc::new(move |_, value: &str| {
                    target_state.borrow_mut().target = value.to_string();
                }),
                on_submit: Rc::new(move |_, value: &str| {
                    target_submit.borrow_mut().target = value.to_string();
                }),
            },
        },
        command_button(
            &state,
            &command,
            "follow",
            "Follow player",
            3,
            follow_enabled,
            |state| ("follow".to_string(), vec![state.target.clone()]),
        ),
        UiControl {
            id: control_id("ui:match:vote"),
            label: "Vote".to_string(),
            rect: Rect {
                x: 64.0,
                y: 232.0,
                width: 496.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Choice {
                choices: vec![
                    UiChoice {
                        id: "map_restart".to_string(),
                        label: "Restart map".to_string(),
                    },
                    UiChoice {
                        id: "nextmap".to_string(),
                        label: "Next map".to_string(),
                    },
                    UiChoice {
                        id: "map".to_string(),
                        label: "Change map".to_string(),
                    },
                    UiChoice {
                        id: "kick".to_string(),
                        label: "Kick player".to_string(),
                    },
                ],
                selected: Some(vote_selected),
                on_select: Rc::new(move |_, value: &str| {
                    vote_state.borrow_mut().vote = value.to_string();
                }),
            },
        },
        command_button(
            &state,
            &command,
            "callvote",
            "Call vote",
            5,
            callvote_enabled,
            |state| {
                if state.vote == "map" || state.vote == "kick" {
                    ("callvote".to_string(), vec![state.vote.clone(), state.target.clone()])
                } else {
                    ("callvote".to_string(), vec![state.vote.clone()])
                }
            },
        ),
        command_button(&state, &command, "yes", "Vote yes", 6, true, |_| {
            ("vote".to_string(), vec!["yes".to_string()])
        }),
        command_button(&state, &command, "no", "Vote no", 7, true, |_| {
            ("vote".to_string(), vec!["no".to_string()])
        }),
        UiControl {
            id: control_id("ui:match:bot"),
            label: "Bot name".to_string(),
            rect: Rect {
                x: 64.0,
                y: 368.0,
                width: 496.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::TextEntry {
                masked: false,
                text: bot_text,
                maximum_length: 64,
                on_change: Rc::new(move |_, value: &str| {
                    bot_state.borrow_mut().bot = value.to_string();
                }),
                on_submit: Rc::new(move |_, value: &str| {
                    bot_submit.borrow_mut().bot = value.to_string();
                }),
            },
        },
        UiControl {
            id: control_id("ui:match:skill"),
            label: "Bot difficulty".to_string(),
            rect: Rect {
                x: 64.0,
                y: 402.0,
                width: 496.0,
                height: 30.0,
            },
            enabled: true,
            visible: true,
            kind: UiControlKind::Choice {
                choices: ["1", "2", "3", "4", "5"]
                    .iter()
                    .map(|id| UiChoice {
                        id: (*id).to_string(),
                        label: (*id).to_string(),
                    })
                    .collect(),
                selected: Some(skill_selected),
                on_select: Rc::new(move |_, value: &str| {
                    skill_state.borrow_mut().skill = value.to_string();
                }),
            },
        },
        command_button(&state, &command, "addbot", "Add bot", 10, bot_enabled, |state| {
            (
                "addbot".to_string(),
                vec![state.bot.clone(), state.skill.clone(), state.team.clone()],
            )
        }),
        command_button(
            &state,
            &command,
            "removebot",
            "Remove named bot",
            11,
            bot_enabled,
            |state| ("kick".to_string(), vec![state.bot.clone()]),
        ),
        match_button(
            "back",
            "Back",
            10,
            true,
            Rc::new(move |_| {
                back_controller.borrow_mut().close_menu();
            }),
        ),
    ];
    UiMenu {
        scroll: Some(UiMenuScroll {
            rect: Rect {
                x: 64.0,
                y: 92.0,
                width: 512.0,
                height: 306.0,
            },
            content_height: 442.0,
            controls: [
                "team",
                "join",
                "target",
                "follow",
                "vote",
                "callvote",
                "yes",
                "no",
                "bot",
                "skill",
                "addbot",
                "removebot",
            ]
            .iter()
            .map(|suffix| control_id(&format!("ui:match:{suffix}")))
            .collect(),
        }),
        id: root.clone(),
        title: "Match controls".to_string(),
        full_screen: true,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Register the match menu factory on the shared controller.
pub fn register_match_menu(controller: &Rc<RefCell<NativeUiController>>, command: CommandFn) -> MatchMenus {
    let root = menu_id(MATCH_MENU_ID);
    let factory_root = root.clone();
    let shared = Rc::new(MatchMenuInputs {
        command,
        controller: Rc::clone(controller),
        state: Rc::new(RefCell::new(MatchMenuState::default())),
    });
    controller
        .borrow_mut()
        .register(root.clone(), Rc::new(move || build_match_menu(&factory_root, &shared)));
    MatchMenus {
        root,
        controller: Rc::clone(controller),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    use crate::ui::common::controller::headless_options;

    type CallLog = Rc<RefCell<Vec<(String, Vec<String>)>>>;

    fn harness() -> (MatchMenuInputs, CallLog, qa_core::identity::SeatId) {
        let owner = IdentityOwner::create("library-match-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat.clone()))));
        let calls: CallLog = Rc::new(RefCell::new(Vec::new()));
        let record = Rc::clone(&calls);
        let inputs = MatchMenuInputs {
            command: Rc::new(move |name: &str, args: &[String]| {
                record.borrow_mut().push((name.to_string(), args.to_vec()));
            }),
            controller,
            state: Rc::new(RefCell::new(MatchMenuState::default())),
        };
        (inputs, calls, seat)
    }

    fn control(menu: &UiMenu, id: &str) -> UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control: {id}"))
            .clone()
    }

    fn activate(menu: &UiMenu, seat: &qa_core::identity::SeatId, id: &str) {
        match &control(menu, id).kind {
            UiControlKind::Button { on_activate } => on_activate(seat.clone()),
            other => panic!("control is not a button: {id} ({other:?})"),
        }
    }

    #[test]
    fn controls_scroll_and_rects_match_donor() {
        let (inputs, _, _) = harness();
        let menu = build_match_menu(&menu_id(MATCH_MENU_ID), &inputs);
        assert_eq!(menu.title, "Match controls");
        assert!(menu.full_screen);
        assert_eq!(menu.controls.len(), 13);
        let scroll = menu.scroll.as_ref().expect("scroll region");
        assert_eq!(
            (scroll.rect.x, scroll.rect.y, scroll.rect.width, scroll.rect.height),
            (64.0, 92.0, 512.0, 306.0)
        );
        assert_eq!(scroll.content_height, 442.0);
        assert_eq!(scroll.controls.len(), 12);
        assert!(!scroll.controls.iter().any(|id| id.as_str() == "ui:match:back"));
        let rows: Vec<(&str, f32)> = [
            ("ui:match:team", 96.0),
            ("ui:match:join", 130.0),
            ("ui:match:target", 164.0),
            ("ui:match:follow", 198.0),
            ("ui:match:vote", 232.0),
            ("ui:match:callvote", 266.0),
            ("ui:match:yes", 300.0),
            ("ui:match:no", 334.0),
            ("ui:match:bot", 368.0),
            ("ui:match:skill", 402.0),
            ("ui:match:addbot", 436.0),
            ("ui:match:removebot", 470.0),
            ("ui:match:back", 436.0),
        ]
        .into_iter()
        .collect();
        for (id, y) in rows {
            assert_eq!(control(&menu, id).rect.y, y, "row y for {id}");
        }
    }

    #[test]
    fn gating_rules_match_donor() {
        let (inputs, _, _) = harness();
        let menu = build_match_menu(&menu_id(MATCH_MENU_ID), &inputs);
        assert!(!control(&menu, "ui:match:follow").enabled);
        assert!(control(&menu, "ui:match:callvote").enabled);
        assert!(control(&menu, "ui:match:addbot").enabled);
        inputs.state.borrow_mut().vote = "kick".to_string();
        inputs.state.borrow_mut().bot = "   ".to_string();
        let menu = build_match_menu(&menu_id(MATCH_MENU_ID), &inputs);
        assert!(!control(&menu, "ui:match:callvote").enabled);
        assert!(!control(&menu, "ui:match:addbot").enabled);
        assert!(!control(&menu, "ui:match:removebot").enabled);
    }

    #[test]
    fn commands_use_current_fields() {
        let (inputs, calls, seat) = harness();
        inputs.state.borrow_mut().team = "blue".to_string();
        inputs.state.borrow_mut().target = "q3dm17".to_string();
        inputs.state.borrow_mut().vote = "map".to_string();
        let menu = build_match_menu(&menu_id(MATCH_MENU_ID), &inputs);
        activate(&menu, &seat, "ui:match:join");
        activate(&menu, &seat, "ui:match:follow");
        activate(&menu, &seat, "ui:match:callvote");
        activate(&menu, &seat, "ui:match:addbot");
        match &control(&menu, "ui:match:team").kind {
            UiControlKind::Choice { on_select, .. } => on_select(seat.clone(), "free"),
            other => panic!("expected a choice: {other:?}"),
        }
        assert_eq!(inputs.state.borrow().team, "free");
        let calls = calls.borrow();
        assert_eq!(calls[0], ("team".to_string(), vec!["blue".to_string()]));
        assert_eq!(calls[1], ("follow".to_string(), vec!["q3dm17".to_string()]));
        assert_eq!(
            calls[2],
            ("callvote".to_string(), vec!["map".to_string(), "q3dm17".to_string()])
        );
        assert_eq!(
            calls[3],
            (
                "addbot".to_string(),
                vec!["sarge".to_string(), "3".to_string(), "blue".to_string()]
            )
        );
    }

    #[test]
    fn register_and_dispose_track_menu() {
        let owner = IdentityOwner::create("library-match-register-test").expect("owner");
        let seat = owner.seat(0);
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(seat))));
        let menus = register_match_menu(&controller, Rc::new(|_, _| {}));
        assert_eq!(menus.root.as_str(), MATCH_MENU_ID);
        assert!(controller.borrow().is_registered(&menus.root));
        let root = menus.root.clone();
        menus.dispose();
        assert!(!controller.borrow().is_registered(&root));
    }
}
