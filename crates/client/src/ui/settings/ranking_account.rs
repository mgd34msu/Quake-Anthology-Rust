//! Ranking account settings menu.
//!
//! Donor provenance: `src/ui/settings/ranking-account.ts` in full. Rows 0-2
//! are the credential fields, row 3 toggles account creation, rows 4-6 hold
//! the submit/reset/spectate buttons, rows 8-10 show the wrapped status, and
//! row 11 goes back. The donor runs account operations asynchronously with a
//! generation counter that drops stale completions; this port runs the
//! synchronous [`RankingAccountActions`](super::rankings::RankingAccountActions)
//! operations inline, so no generation guard is needed.

use std::cell::RefCell;
use std::rc::Rc;

use super::rankings::{
    ranking_account_view, RankingAccountActions, RankingAccountRequest, RankingAccountView, RankingPlayerView,
};
use super::{control_id, menu_id};
use crate::ui::common::controller::NativeUiController;
use crate::ui::common::layout::{menu_row, MenuRowOptions};
use crate::ui::types::{SeatUiController as _, UiControl, UiControlKind, UiMenu, UiMenuId};

/// Source slot behind the menu, if one is admitted for this seat.
pub type RankingAccountSource = Rc<dyn Fn() -> Option<Rc<RefCell<dyn RankingAccountActions>>>>;

/// Form state shared by the menu factory and every control callback.
struct FormState {
    username: String,
    password: String,
    email: String,
    create: bool,
    busy: bool,
    message: String,
}

impl FormState {
    fn new() -> Self {
        Self {
            username: String::new(),
            password: String::new(),
            email: String::new(),
            create: false,
            busy: false,
            message: String::new(),
        }
    }

    /// Clear the credential fields, keeping the create toggle.
    fn clear_credentials(&mut self) {
        self.username.clear();
        self.password.clear();
        self.email.clear();
    }

    /// Reset the transient state when the menu closes or disposes.
    fn close(&mut self) {
        self.clear_credentials();
        self.busy = false;
        self.message.clear();
    }
}

/// Run one synchronous account operation, recording failures as the status.
fn run_request(form: &mut FormState, operation: impl FnOnce() -> Result<(), String>) {
    if form.busy {
        return;
    }
    form.busy = true;
    form.message.clear();
    if let Err(error) = operation() {
        form.message = error;
    }
    form.busy = false;
}

/// Select the status line, mirroring the donor fallback chain exactly.
fn status_text(message: &str, busy: bool, view: Option<&RankingAccountView>) -> String {
    if !message.is_empty() {
        return message.to_string();
    }
    if busy {
        return "Working...".to_string();
    }
    let Some(view) = view else {
        return "No ranking account service for this player.".to_string();
    };
    match view {
        RankingAccountView::Unavailable { message } => message.clone(),
        RankingAccountView::Disabled => "Rankings are disabled for this match.".to_string(),
        RankingAccountView::Busy => "Contacting ranking provider...".to_string(),
        RankingAccountView::Account { player } => match player {
            RankingPlayerView::Pending => "Contacting ranking provider...".to_string(),
            RankingPlayerView::Active { account } => {
                format!("Signed in. Rank: {}", account.rank)
            }
            RankingPlayerView::Denied { reason } => reason.clone(),
            RankingPlayerView::Spectator => "Spectating. Sign in to play ranked.".to_string(),
            RankingPlayerView::Idle => "Sign in or create an account.".to_string(),
        },
    }
}

/// Wrap a status line the way the donor `/.{1,64}(?:\s|$)|.{1,64}/g` match
/// does: up to 64 characters plus one consumed trailing whitespace, else a
/// hard 64-character cut. Callers cap the result at three lines.
fn wrap_status(text: &str) -> Vec<String> {
    const WIDTH: usize = 64;
    let mut lines = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.chars().count() <= WIDTH {
            lines.push(rest.to_string());
            break;
        }
        let mut split: Option<usize> = None;
        for (index, (offset, glyph)) in rest.char_indices().enumerate() {
            if index > WIDTH {
                break;
            }
            if index > 0 && glyph.is_whitespace() {
                split = Some(offset + glyph.len_utf8());
            }
        }
        match split {
            Some(end) => {
                lines.push(rest[..end].to_string());
                rest = &rest[end..];
            }
            None => {
                let end = rest.char_indices().nth(WIDTH).map_or(rest.len(), |(offset, _)| offset);
                lines.push(rest[..end].to_string());
                rest = &rest[end..];
            }
        }
    }
    lines
}

/// Whether the credential fields and create toggle accept input.
fn editable(busy: bool, player: Option<&RankingPlayerView>) -> bool {
    !busy
        && matches!(
            player,
            Some(RankingPlayerView::Idle | RankingPlayerView::Spectator | RankingPlayerView::Denied { .. })
        )
}

fn button(
    id: &str,
    label: String,
    row: i32,
    enabled: bool,
    on_activate: Rc<dyn Fn(qa_core::identity::SeatId)>,
) -> UiControl {
    UiControl {
        id: control_id(id),
        label,
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled,
        visible: true,
        kind: UiControlKind::Button { on_activate },
    }
}

/// Arguments for one account text field.
struct FieldArgs<'a> {
    id: &'a str,
    label: &'a str,
    row: i32,
    text: String,
    masked: bool,
    enabled: bool,
    form: &'a Rc<RefCell<FormState>>,
    select: fn(&mut FormState) -> &mut String,
}

fn field(args: FieldArgs<'_>) -> UiControl {
    let FieldArgs {
        id,
        label,
        row,
        text,
        masked,
        enabled,
        form,
        select,
    } = args;
    let change_form = Rc::clone(form);
    UiControl {
        id: control_id(id),
        label: label.to_string(),
        rect: menu_row(row, &MenuRowOptions::default()),
        enabled,
        visible: true,
        kind: UiControlKind::TextEntry {
            masked,
            text,
            maximum_length: 128,
            on_change: Rc::new(move |_, value| {
                *select(&mut change_form.borrow_mut()) = value.to_string();
            }),
            on_submit: Rc::new(|_, _| {}),
        },
    }
}

/// Build one fresh menu snapshot; the registered factory calls this on every
/// input event and draw so the rows track live service state.
fn build_menu(
    root: &UiMenuId,
    form: &Rc<RefCell<FormState>>,
    current: &RankingAccountSource,
    controller: &Rc<RefCell<NativeUiController>>,
) -> UiMenu {
    let actions = current();
    let view: Option<RankingAccountView> = actions.as_ref().map(|slot| {
        let borrowed = slot.borrow();
        ranking_account_view(&*borrowed)
    });
    let player: Option<RankingPlayerView> = match view.as_ref() {
        Some(RankingAccountView::Account { player }) => Some(player.clone()),
        _ => None,
    };
    let snapshot = form.borrow();
    let username = snapshot.username.clone();
    let password = snapshot.password.clone();
    let email = snapshot.email.clone();
    let create = snapshot.create;
    let busy = snapshot.busy;
    let message = snapshot.message.clone();
    drop(snapshot);
    let can_edit = editable(busy, player.as_ref());
    let status = status_text(&message, busy, view.as_ref());

    let mut controls = Vec::new();
    controls.push(field(FieldArgs {
        id: "ui:rankings:username",
        label: "Username",
        row: 0,
        text: username.clone(),
        masked: false,
        enabled: can_edit,
        form,
        select: |form: &mut FormState| &mut form.username,
    }));
    controls.push(field(FieldArgs {
        id: "ui:rankings:password",
        label: "Password",
        row: 1,
        text: password.clone(),
        masked: true,
        enabled: can_edit,
        form,
        select: |form: &mut FormState| &mut form.password,
    }));
    controls.push(field(FieldArgs {
        id: "ui:rankings:email",
        label: "Email (new account)",
        row: 2,
        text: email.clone(),
        masked: false,
        enabled: can_edit,
        form,
        select: |form: &mut FormState| &mut form.email,
    }));

    let toggle_form = Rc::clone(form);
    controls.push(UiControl {
        id: control_id("ui:rankings:create"),
        label: "Create a new account".to_string(),
        rect: menu_row(3, &MenuRowOptions::default()),
        enabled: can_edit,
        visible: true,
        kind: UiControlKind::Toggle {
            checked: create,
            on_change: Rc::new(move |_, value| {
                toggle_form.borrow_mut().create = value;
            }),
        },
    });

    let submit_form = Rc::clone(form);
    let submit_current = Rc::clone(current);
    let submit_enabled =
        can_edit && !username.trim().is_empty() && !password.is_empty() && (!create || !email.trim().is_empty());
    let submit_label = if create {
        "Create account".to_string()
    } else {
        "Sign in".to_string()
    };
    controls.push(button(
        "ui:rankings:submit",
        submit_label,
        4,
        submit_enabled,
        Rc::new(move |_| {
            let Some(slot) = submit_current() else {
                return;
            };
            let (username, password, email, create) = {
                let form = submit_form.borrow();
                (
                    form.username.clone(),
                    form.password.clone(),
                    form.email.clone(),
                    form.create,
                )
            };
            if !can_edit || username.trim().is_empty() || password.is_empty() {
                return;
            }
            let request = if create {
                RankingAccountRequest::Create {
                    username,
                    password,
                    email,
                }
            } else {
                RankingAccountRequest::Login { username, password }
            };
            let mut form = submit_form.borrow_mut();
            form.password.clear();
            run_request(&mut form, || slot.borrow_mut().submit(request));
        }),
    ));

    let reset_form = Rc::clone(form);
    let reset_current = Rc::clone(current);
    let reset_enabled = !busy
        && matches!(
            player,
            Some(RankingPlayerView::Denied { .. } | RankingPlayerView::Spectator)
        );
    controls.push(button(
        "ui:rankings:reset",
        "Reset account status".to_string(),
        5,
        reset_enabled,
        Rc::new(move |_| {
            let Some(slot) = reset_current() else {
                return;
            };
            run_request(&mut reset_form.borrow_mut(), || slot.borrow_mut().reset());
        }),
    ));

    let spectate_form = Rc::clone(form);
    let spectate_current = Rc::clone(current);
    let spectate_enabled = !busy && player.is_some() && !matches!(player, Some(RankingPlayerView::Pending));
    controls.push(button(
        "ui:rankings:spectate",
        "Spectate / sign out".to_string(),
        6,
        spectate_enabled,
        Rc::new(move |_| {
            spectate_form.borrow_mut().clear_credentials();
            let Some(slot) = spectate_current() else {
                return;
            };
            run_request(&mut spectate_form.borrow_mut(), || slot.borrow_mut().spectate());
        }),
    ));

    let back_controller = Rc::clone(controller);
    controls.push(button(
        "ui:rankings:back",
        "Back".to_string(),
        11,
        true,
        Rc::new(move |_| {
            back_controller.borrow_mut().close_menu();
        }),
    ));

    for (index, line) in wrap_status(&status).iter().take(3).enumerate() {
        controls.push(button(
            &format!("ui:rankings:status-{index}"),
            line.clone(),
            8 + index as i32,
            false,
            Rc::new(|_| {}),
        ));
    }

    let close_form = Rc::clone(form);
    let menu_root = root.clone();
    UiMenu {
        scroll: None,
        id: menu_root,
        title: "Ranking account".to_string(),
        full_screen: false,
        controls,
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(move |_| {
            close_form.borrow_mut().close();
        }),
    }
}

/// Open ranking account menu: the root plus its shared form state.
pub struct RankingMenus {
    /// Menu root id (`menu:rankings:account`).
    pub root: UiMenuId,
    controller: Rc<RefCell<NativeUiController>>,
    form: Rc<RefCell<FormState>>,
}

impl RankingMenus {
    /// Clear the transient form state, then unregister the menu.
    pub fn dispose(self) {
        self.form.borrow_mut().close();
        self.controller.borrow_mut().unregister(&self.root);
    }
}

impl std::fmt::Debug for RankingMenus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RankingMenus")
            .field("root", &self.root)
            .finish_non_exhaustive()
    }
}

/// Register the ranking account menu.
///
/// The factory captures shared handles only; control callbacks re-enter the
/// controller through the shared handle, so callers must not hold a borrow
/// across input or draw calls that activate those controls.
pub fn register_ranking_account_menu(
    controller: &Rc<RefCell<NativeUiController>>,
    current: RankingAccountSource,
) -> RankingMenus {
    let root = menu_id("menu:rankings:account");
    let form = Rc::new(RefCell::new(FormState::new()));
    let factory_form = Rc::clone(&form);
    let factory_current = Rc::clone(&current);
    let factory_controller = Rc::clone(controller);
    let factory_root = root.clone();
    controller.borrow_mut().register(
        root.clone(),
        Rc::new(move || build_menu(&factory_root, &factory_form, &factory_current, &factory_controller)),
    );
    RankingMenus {
        root,
        controller: Rc::clone(controller),
        form,
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::{IdentityOwner, SeatId};

    use super::super::rankings::{RankingAccount, RankingServiceView};
    use crate::ui::common::controller::headless_options;
    use crate::ui::types::UiControlKind;

    use super::*;

    struct FakeActions {
        service: RankingServiceView,
        player: RankingPlayerView,
        submitted: Vec<RankingAccountRequest>,
        resets: u32,
        spectates: u32,
        submit_result: Result<(), String>,
        reset_result: Result<(), String>,
        spectate_result: Result<(), String>,
    }

    impl FakeActions {
        fn new(service: RankingServiceView, player: RankingPlayerView) -> Self {
            Self {
                service,
                player,
                submitted: Vec::new(),
                resets: 0,
                spectates: 0,
                submit_result: Ok(()),
                reset_result: Ok(()),
                spectate_result: Ok(()),
            }
        }
    }

    impl RankingAccountActions for FakeActions {
        fn service(&self) -> RankingServiceView {
            self.service.clone()
        }

        fn player(&self) -> RankingPlayerView {
            self.player.clone()
        }

        fn submit(&mut self, request: RankingAccountRequest) -> Result<(), String> {
            self.submitted.push(request);
            self.submit_result.clone()
        }

        fn reset(&mut self) -> Result<(), String> {
            self.resets += 1;
            self.reset_result.clone()
        }

        fn spectate(&mut self) -> Result<(), String> {
            self.spectates += 1;
            self.spectate_result.clone()
        }
    }

    fn seat() -> SeatId {
        IdentityOwner::create("ranking-test").unwrap().seat(0)
    }

    fn harness(
        service: RankingServiceView,
        player: RankingPlayerView,
    ) -> (
        Rc<RefCell<NativeUiController>>,
        Rc<RefCell<FakeActions>>,
        RankingAccountSource,
    ) {
        let owner = IdentityOwner::create("ranking-test").unwrap();
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(owner.seat(0)))));
        let fake = Rc::new(RefCell::new(FakeActions::new(service, player)));
        let slot: Rc<RefCell<dyn RankingAccountActions>> = fake.clone();
        let current: RankingAccountSource = Rc::new(move || Some(Rc::clone(&slot)));
        (controller, fake, current)
    }

    fn menu_for(
        controller: &Rc<RefCell<NativeUiController>>,
        current: &RankingAccountSource,
        setup: impl FnOnce(&mut FormState),
    ) -> UiMenu {
        let form = Rc::new(RefCell::new(FormState::new()));
        setup(&mut form.borrow_mut());
        build_menu(&menu_id("menu:rankings:account"), &form, current, controller)
    }

    fn control<'menu>(menu: &'menu UiMenu, id: &str) -> &'menu UiControl {
        menu.controls
            .iter()
            .find(|control| control.id.as_str() == id)
            .unwrap_or_else(|| panic!("missing control {id}"))
    }

    fn activate(menu: &UiMenu, id: &str) {
        match &control(menu, id).kind {
            UiControlKind::Button { on_activate } => on_activate(seat()),
            kind => panic!("control {id} is not a button: {kind:?}"),
        }
    }

    fn status_lines(menu: &UiMenu) -> Vec<String> {
        menu.controls
            .iter()
            .filter(|control| control.id.as_str().starts_with("ui:rankings:status-"))
            .map(|control| control.label.clone())
            .collect()
    }

    fn account(rank: i32) -> RankingAccount {
        RankingAccount { player_id: 9, rank }
    }

    #[test]
    fn menu_rows_follow_player_state() {
        let cases = vec![
            (RankingPlayerView::Idle, true, false, true),
            (RankingPlayerView::Spectator, true, true, true),
            (RankingPlayerView::Pending, false, false, false),
            (RankingPlayerView::Active { account: account(7) }, false, false, true),
            (
                RankingPlayerView::Denied {
                    reason: "Bad password.".to_string(),
                },
                true,
                true,
                true,
            ),
        ];
        for (player, can_edit, can_reset, can_spectate) in cases {
            let (controller, _, current) = harness(RankingServiceView::Active, player);
            let menu = menu_for(&controller, &current, |form| {
                form.username = "quake".to_string();
                form.password = "secret".to_string();
                form.email = "q@example.com".to_string();
            });
            assert_eq!(menu.id.as_str(), "menu:rankings:account");
            assert_eq!(menu.title, "Ranking account");
            assert!(!menu.full_screen);
            assert_eq!(control(&menu, "ui:rankings:username").enabled, can_edit);
            assert_eq!(control(&menu, "ui:rankings:password").enabled, can_edit);
            assert_eq!(control(&menu, "ui:rankings:email").enabled, can_edit);
            assert_eq!(control(&menu, "ui:rankings:create").enabled, can_edit);
            assert_eq!(control(&menu, "ui:rankings:submit").enabled, can_edit);
            assert_eq!(control(&menu, "ui:rankings:reset").enabled, can_reset);
            assert_eq!(control(&menu, "ui:rankings:spectate").enabled, can_spectate);
            assert!(control(&menu, "ui:rankings:back").enabled);
            assert_eq!(
                control(&menu, "ui:rankings:username").rect,
                menu_row(0, &MenuRowOptions::default())
            );
            assert_eq!(
                control(&menu, "ui:rankings:back").rect,
                menu_row(11, &MenuRowOptions::default())
            );
        }
    }

    #[test]
    fn submit_validation_gates_empty_fields_and_email() {
        let (controller, _, current) = harness(RankingServiceView::Active, RankingPlayerView::Idle);
        let enabled = |setup: &dyn Fn(&mut FormState)| {
            let form = Rc::new(RefCell::new(FormState::new()));
            setup(&mut form.borrow_mut());
            build_menu(&menu_id("menu:rankings:account"), &form, &current, &controller)
        };
        let blank: &dyn Fn(&mut FormState) = &|_| {};
        assert!(!control(&enabled(blank), "ui:rankings:submit").enabled);
        let no_password: &dyn Fn(&mut FormState) = &|form| {
            form.username = "quake".to_string();
        };
        assert!(!control(&enabled(no_password), "ui:rankings:submit").enabled);
        let whitespace_user: &dyn Fn(&mut FormState) = &|form| {
            form.username = "   ".to_string();
            form.password = "secret".to_string();
        };
        assert!(!control(&enabled(whitespace_user), "ui:rankings:submit").enabled);
        let login: &dyn Fn(&mut FormState) = &|form| {
            form.username = "quake".to_string();
            form.password = "secret".to_string();
        };
        let menu = enabled(login);
        assert!(control(&menu, "ui:rankings:submit").enabled);
        assert_eq!(control(&menu, "ui:rankings:submit").label, "Sign in");
        let create_no_email: &dyn Fn(&mut FormState) = &|form| {
            form.username = "quake".to_string();
            form.password = "secret".to_string();
            form.create = true;
        };
        let menu = enabled(create_no_email);
        assert!(!control(&menu, "ui:rankings:submit").enabled);
        assert_eq!(control(&menu, "ui:rankings:submit").label, "Create account");
        let create: &dyn Fn(&mut FormState) = &|form| {
            form.username = "quake".to_string();
            form.password = "secret".to_string();
            form.email = "q@example.com".to_string();
            form.create = true;
        };
        assert!(control(&enabled(create), "ui:rankings:submit").enabled);
    }

    #[test]
    fn submit_sends_request_clears_password_and_reports_errors() {
        let (controller, fake, current) = harness(RankingServiceView::Active, RankingPlayerView::Idle);
        let form = Rc::new(RefCell::new(FormState::new()));
        {
            let mut form = form.borrow_mut();
            form.username = "quake".to_string();
            form.password = "secret".to_string();
        }
        let menu = build_menu(&menu_id("menu:rankings:account"), &form, &current, &controller);
        activate(&menu, "ui:rankings:submit");
        assert_eq!(
            fake.borrow().submitted,
            vec![RankingAccountRequest::Login {
                username: "quake".to_string(),
                password: "secret".to_string(),
            }]
        );
        assert_eq!(form.borrow().username, "quake");
        assert!(form.borrow().password.is_empty());
        assert!(!form.borrow().busy);

        fake.borrow_mut().submit_result = Err("Provider refused.".to_string());
        {
            let mut form = form.borrow_mut();
            form.password = "again".to_string();
            form.create = true;
            form.email = "q@example.com".to_string();
        }
        let menu = build_menu(&menu_id("menu:rankings:account"), &form, &current, &controller);
        activate(&menu, "ui:rankings:submit");
        assert_eq!(fake.borrow().submitted.len(), 2);
        assert_eq!(
            fake.borrow().submitted[1],
            RankingAccountRequest::Create {
                username: "quake".to_string(),
                password: "again".to_string(),
                email: "q@example.com".to_string(),
            }
        );
        assert_eq!(form.borrow().message, "Provider refused.");
        let menu = build_menu(&menu_id("menu:rankings:account"), &form, &current, &controller);
        assert_eq!(status_lines(&menu), vec!["Provider refused.".to_string()]);
    }

    #[test]
    fn reset_and_spectate_clear_and_call_through() {
        let (controller, fake, current) = harness(
            RankingServiceView::Active,
            RankingPlayerView::Denied {
                reason: "Bad password.".to_string(),
            },
        );
        let form = Rc::new(RefCell::new(FormState::new()));
        {
            let mut form = form.borrow_mut();
            form.username = "quake".to_string();
            form.password = "secret".to_string();
            form.email = "q@example.com".to_string();
        }
        let menu = build_menu(&menu_id("menu:rankings:account"), &form, &current, &controller);
        activate(&menu, "ui:rankings:reset");
        assert_eq!(fake.borrow().resets, 1);
        activate(&menu, "ui:rankings:spectate");
        assert_eq!(fake.borrow().spectates, 1);
        assert!(form.borrow().username.is_empty());
        assert!(form.borrow().password.is_empty());
        assert!(form.borrow().email.is_empty());
    }

    #[test]
    fn busy_disables_every_action_row() {
        let (controller, _, current) = harness(RankingServiceView::Active, RankingPlayerView::Spectator);
        let menu = menu_for(&controller, &current, |form| {
            form.username = "quake".to_string();
            form.password = "secret".to_string();
            form.busy = true;
        });
        for id in [
            "ui:rankings:username",
            "ui:rankings:password",
            "ui:rankings:email",
            "ui:rankings:create",
            "ui:rankings:submit",
            "ui:rankings:reset",
            "ui:rankings:spectate",
        ] {
            assert!(!control(&menu, id).enabled, "{id} stays enabled while busy");
        }
        assert_eq!(status_lines(&menu), vec!["Working...".to_string()]);
    }

    #[test]
    fn status_text_covers_every_view_state() {
        let (controller, _, current) = harness(RankingServiceView::Disabled, RankingPlayerView::Idle);
        let menu = menu_for(&controller, &current, |_| {});
        assert!(!control(&menu, "ui:rankings:username").enabled);
        assert_eq!(
            status_lines(&menu),
            vec!["Rankings are disabled for this match.".to_string()]
        );

        let (controller, _, current) = harness(
            RankingServiceView::Unavailable {
                message: "No provider configured.".to_string(),
            },
            RankingPlayerView::Idle,
        );
        let menu = menu_for(&controller, &current, |_| {});
        assert_eq!(status_lines(&menu), vec!["No provider configured.".to_string()]);

        let (controller, _, current) = harness(RankingServiceView::Busy, RankingPlayerView::Idle);
        let menu = menu_for(&controller, &current, |_| {});
        assert_eq!(status_lines(&menu), vec!["Contacting ranking provider...".to_string()]);

        let owner = IdentityOwner::create("ranking-test").unwrap();
        let controller = Rc::new(RefCell::new(NativeUiController::new(headless_options(owner.seat(0)))));
        let current: RankingAccountSource = Rc::new(|| None);
        let menu = menu_for(&controller, &current, |_| {});
        assert!(!control(&menu, "ui:rankings:spectate").enabled);
        assert_eq!(
            status_lines(&menu),
            vec!["No ranking account service for this player.".to_string()]
        );

        let (controller, _, current) = harness(
            RankingServiceView::Active,
            RankingPlayerView::Active { account: account(42) },
        );
        let menu = menu_for(&controller, &current, |_| {});
        assert_eq!(status_lines(&menu), vec!["Signed in. Rank: 42".to_string()]);

        let (controller, _, current) = harness(RankingServiceView::Active, RankingPlayerView::Spectator);
        let menu = menu_for(&controller, &current, |_| {});
        assert_eq!(
            status_lines(&menu),
            vec!["Spectating. Sign in to play ranked.".to_string()]
        );

        let (controller, _, current) = harness(RankingServiceView::Active, RankingPlayerView::Idle);
        let menu = menu_for(&controller, &current, |_| {});
        assert_eq!(status_lines(&menu), vec!["Sign in or create an account.".to_string()]);
    }

    #[test]
    fn status_wrapping_matches_donor_width_and_caps_at_three_lines() {
        assert!(wrap_status("").is_empty());
        assert_eq!(wrap_status("Signed in. Rank: 7"), vec!["Signed in. Rank: 7"]);
        let exact = "r".repeat(64);
        assert_eq!(wrap_status(&exact), vec![exact.clone()]);
        let long = "r".repeat(70);
        assert_eq!(wrap_status(&long), vec!["r".repeat(64), "r".repeat(6)]);
        let words = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu";
        let lines = wrap_status(words);
        assert!(lines.iter().all(|line| line.chars().count() <= 65));
        assert_eq!(lines.concat(), words);

        let reason = "word ".repeat(60);
        let wrapped = wrap_status(&reason);
        assert!(wrapped.len() > 3);
        assert_eq!(wrapped.concat(), reason);
        assert!(wrapped.iter().all(|line| line.chars().count() <= 65));
        let (controller, _, current) = harness(
            RankingServiceView::Active,
            RankingPlayerView::Denied { reason: reason.clone() },
        );
        let menu = menu_for(&controller, &current, |_| {});
        let lines = status_lines(&menu);
        assert_eq!(lines, wrapped[..3].to_vec());
        for (index, line) in lines.iter().enumerate() {
            let id = format!("ui:rankings:status-{index}");
            let status = control(&menu, &id);
            assert_eq!(status.label, *line);
            assert!(!status.enabled);
            assert_eq!(status.rect, menu_row(8 + index as i32, &MenuRowOptions::default()));
        }
    }

    #[test]
    fn close_clears_transient_state_but_keeps_create_toggle() {
        let (controller, _, current) = harness(RankingServiceView::Active, RankingPlayerView::Idle);
        let form = Rc::new(RefCell::new(FormState::new()));
        {
            let mut form = form.borrow_mut();
            form.username = "quake".to_string();
            form.password = "secret".to_string();
            form.email = "q@example.com".to_string();
            form.create = true;
            form.busy = true;
            form.message = "stale".to_string();
        }
        let menu = build_menu(&menu_id("menu:rankings:account"), &form, &current, &controller);
        (menu.on_close)(seat());
        let form = form.borrow();
        assert!(form.username.is_empty());
        assert!(form.password.is_empty());
        assert!(form.email.is_empty());
        assert!(form.create);
        assert!(!form.busy);
        assert!(form.message.is_empty());
    }

    #[test]
    fn register_and_dispose_round_trip() {
        let (controller, _, current) = harness(RankingServiceView::Active, RankingPlayerView::Idle);
        let probe = Rc::clone(&current);
        let menus = register_ranking_account_menu(&controller, current);
        assert_eq!(menus.root.as_str(), "menu:rankings:account");
        assert!(controller.borrow().is_registered(&menus.root));
        controller.borrow_mut().open_menu(&menus.root).unwrap();
        assert_eq!(controller.borrow().active_menu().as_ref(), Some(&menus.root));
        let form = Rc::new(RefCell::new(FormState::new()));
        let menu = build_menu(&menus.root, &form, &probe, &controller);
        activate(&menu, "ui:rankings:back");
        assert_eq!(controller.borrow().active_menu(), None);
        menus.dispose();
        let root = menu_id("menu:rankings:account");
        assert!(!controller.borrow().is_registered(&root));
    }
}
