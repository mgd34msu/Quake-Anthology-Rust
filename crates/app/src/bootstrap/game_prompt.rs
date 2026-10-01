//! Seat-local game prompts (QuakeC-driven choice menus).
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/game-prompt.ts`
//! (`gamePromptMenu`, `SeatGamePrompt`). Synchronous port: catalogs load
//! through [`GamePromptCatalogs`], menus through [`GamePromptController`]
//! (`qa-client`'s controller does not expose menu open/close yet, so the
//! surface is a local trait).

use qa_client::ui::common::layout::{menu_row, MenuRowOptions};
use qa_client::ui::types::{
    SeatInputEvent, SeatInputEventKind, SeatInputFocus, UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId,
};
use qa_content::contract::ContentId;
use qa_core::identity::{ActorId, SeatId};
use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use thiserror::Error;

/// Menu id for the seat game prompt.
pub const GAME_PROMPT_MENU: &str = "menu:application:prompt";

/// Failure of game-prompt preparation.
#[derive(Debug, Error)]
pub enum GamePromptError {
    /// Catalog loading failed.
    #[error("{0}")]
    Catalog(String),
    /// Menu id is invalid.
    #[error(transparent)]
    Ui(#[from] qa_client::ClientError),
}

/// One prompt choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GamePromptChoice {
    /// Display label.
    pub label: String,
    /// Impulse fired on selection.
    pub impulse: i32,
}

/// Composition events consumed by the prompt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GamePromptEvent {
    /// Show a prompt.
    Prompt {
        /// Target actor.
        actor: ActorId,
        /// Source content.
        content: ContentId,
        /// Prompt title.
        title: String,
        /// Prompt choices.
        choices: Vec<GamePromptChoice>,
    },
    /// Clear the prompt.
    ClearPrompt {
        /// Target actor.
        actor: ActorId,
    },
}

/// Localization catalog loading for prompts.
pub trait GamePromptCatalogs {
    /// Load the `content`/`language` catalog (keyed source -> localized).
    fn load_catalog(&mut self, content: &ContentId, language: &str)
        -> Result<HashMap<String, String>, GamePromptError>;
}

/// Menu controller surface used by the prompt.
pub trait GamePromptController {
    /// Register the prompt menu factory.
    fn register_prompt(&mut self, factory: Rc<dyn Fn() -> UiMenu>);
    /// Open the prompt menu.
    fn open_prompt(&mut self);
    /// Close the prompt menu.
    fn close_prompt(&mut self);
    /// Whether the prompt menu is active.
    fn prompt_active(&self) -> bool;
    /// Unregister the prompt menu.
    fn unregister_prompt(&mut self);
}

struct PendingPrompt {
    actor: ActorId,
    content: ContentId,
    title: String,
    choices: Vec<GamePromptChoice>,
    prepared: bool,
}

struct PromptInner {
    seat: SeatId,
    actor: Box<dyn Fn() -> ActorId>,
    impulse: Box<dyn FnMut(i32)>,
    language: Box<dyn Fn() -> String>,
    pending: Option<PendingPrompt>,
    prepared_title: String,
    prepared_language: String,
    page: usize,
    choices: Vec<GamePromptChoice>,
    catalogs: HashMap<String, HashMap<String, String>>,
    queue: Vec<PromptIntent>,
}

enum PromptIntent {
    Choose(usize),
    Page(i64),
}

impl PromptInner {
    fn view(&self) -> (String, usize, Vec<GamePromptChoice>) {
        (self.prepared_title.clone(), self.page, self.choices.clone())
    }

    fn choose(&mut self, index: usize) {
        let current = (self.actor)();
        let ready = self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.prepared && pending.actor == current);
        if !ready {
            return;
        }
        let Some(choice) = self.choices.get(index).cloned() else {
            return;
        };
        self.clear_state();
        (self.impulse)(choice.impulse);
    }

    fn clear_state(&mut self) {
        self.pending = None;
        self.prepared_title.clear();
        self.page = 0;
        self.choices.clear();
    }

    fn drain(&mut self) {
        for intent in std::mem::take(&mut self.queue) {
            match intent {
                PromptIntent::Choose(index) => self.choose(index),
                PromptIntent::Page(delta) => {
                    if delta < 0 {
                        self.page = self.page.saturating_sub((-delta) as usize);
                    } else {
                        self.page += delta as usize;
                    }
                }
            }
        }
    }
}

fn prompt_controls(inner: &Rc<RefCell<PromptInner>>) -> Vec<UiControl> {
    let (title_ignored, page, choices) = inner.borrow().view();
    let _ = title_ignored;
    let mut controls = Vec::new();
    let first = page * 4;
    for (row, choice) in choices.iter().skip(first).take(4).enumerate() {
        let index = first + row;
        let queued = Rc::clone(inner);
        controls.push(UiControl {
            id: UiControlId::new(&format!("ui:game-prompt:{index}")).expect("prompt control id"),
            label: format!("{}. {}", index + 1, choice.label),
            rect: menu_row(
                row as i32,
                &MenuRowOptions {
                    x: None,
                    y: Some(220.0),
                    width: None,
                    height: Some(44.0),
                },
            ),
            enabled: true,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    queued.borrow_mut().queue.push(PromptIntent::Choose(index));
                }),
            },
        });
    }
    if choices.len() > 4 {
        let pages = choices.len().div_ceil(4);
        let previous = Rc::clone(inner);
        controls.push(UiControl {
            id: UiControlId::new("ui:game-prompt:previous").expect("prompt control id"),
            label: "Previous".to_owned(),
            rect: menu_row(
                0,
                &MenuRowOptions {
                    x: None,
                    y: Some(408.0),
                    width: Some(248.0),
                    height: None,
                },
            ),
            enabled: page > 0,
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    previous.borrow_mut().queue.push(PromptIntent::Page(-1));
                }),
            },
        });
        let next = Rc::clone(inner);
        controls.push(UiControl {
            id: UiControlId::new("ui:game-prompt:next").expect("prompt control id"),
            label: format!("Next ({}/{pages})", page + 1),
            rect: menu_row(
                0,
                &MenuRowOptions {
                    x: Some(328.0),
                    y: Some(408.0),
                    width: Some(248.0),
                    height: None,
                },
            ),
            enabled: first + 4 < choices.len(),
            visible: true,
            kind: UiControlKind::Button {
                on_activate: Rc::new(move |_| {
                    next.borrow_mut().queue.push(PromptIntent::Page(1));
                }),
            },
        });
    }
    controls
}

fn prompt_menu(inner: &Rc<RefCell<PromptInner>>) -> UiMenu {
    let (title, _, _) = inner.borrow().view();
    UiMenu {
        scroll: None,
        id: UiMenuId::new(GAME_PROMPT_MENU).expect("prompt menu id"),
        title,
        full_screen: false,
        controls: prompt_controls(inner),
        on_open: Rc::new(|_| {}),
        on_close: Rc::new(|_| {}),
    }
}

/// Seat-local QuakeC game prompt.
pub struct SeatGamePrompt<C> {
    controller: C,
    inner: Rc<RefCell<PromptInner>>,
}

impl<C: GamePromptController> SeatGamePrompt<C> {
    /// Create a prompt for `seat`, registering its menu factory.
    pub fn new(
        seat: SeatId,
        actor: Box<dyn Fn() -> ActorId>,
        mut controller: C,
        impulse: Box<dyn FnMut(i32)>,
        language: Box<dyn Fn() -> String>,
    ) -> Self {
        let inner = Rc::new(RefCell::new(PromptInner {
            seat,
            actor,
            impulse,
            language,
            pending: None,
            prepared_title: String::new(),
            prepared_language: String::new(),
            page: 0,
            choices: Vec::new(),
            catalogs: HashMap::new(),
            queue: Vec::new(),
        }));
        let factory_inner = Rc::clone(&inner);
        controller.register_prompt(Rc::new(move || prompt_menu(&factory_inner)));
        Self { controller, inner }
    }

    /// Borrow the controller.
    #[must_use]
    pub fn controller(&self) -> &C {
        &self.controller
    }

    /// Mutably borrow the controller.
    pub fn controller_mut(&mut self) -> &mut C {
        &mut self.controller
    }

    /// Consume composition events.
    pub fn receive(&mut self, events: &[GamePromptEvent]) {
        for event in events {
            match event {
                GamePromptEvent::ClearPrompt { actor } => {
                    if *actor == (self.inner.borrow().actor)() {
                        self.clear();
                    }
                }
                GamePromptEvent::Prompt {
                    actor,
                    content,
                    title,
                    choices,
                } => {
                    if *actor == (self.inner.borrow().actor)() {
                        self.inner.borrow_mut().pending = Some(PendingPrompt {
                            actor: actor.clone(),
                            content: content.clone(),
                            title: title.clone(),
                            choices: choices.clone(),
                            prepared: false,
                        });
                    }
                }
            }
        }
    }

    /// Prepare the pending prompt and sync the menu with seat focus.
    pub fn prepare<A: GamePromptCatalogs>(
        &mut self,
        catalogs: &mut A,
        focus: &dyn Fn() -> SeatInputFocus,
    ) -> Result<(), GamePromptError> {
        self.inner.borrow_mut().drain();
        let language = (self.inner.borrow().language)();
        let stale_actor = {
            let inner = self.inner.borrow();
            inner
                .pending
                .as_ref()
                .is_some_and(|pending| (inner.actor)() != pending.actor)
        };
        if stale_actor {
            self.clear();
            return Ok(());
        }
        let needs_prepare = {
            let inner = self.inner.borrow();
            inner
                .pending
                .as_ref()
                .is_some_and(|pending| !pending.prepared || language != inner.prepared_language)
        };
        if needs_prepare {
            let (content, title, choices) = {
                let inner = self.inner.borrow();
                let pending = inner.pending.as_ref().expect("pending checked");
                (pending.content.clone(), pending.title.clone(), pending.choices.clone())
            };
            let key = format!("{}:{language}", content.as_str());
            if !self.inner.borrow().catalogs.contains_key(&key) {
                let catalog = catalogs.load_catalog(&content, &language)?;
                self.inner.borrow_mut().catalogs.insert(key.clone(), catalog);
            }
            if (self.inner.borrow().language)() != language {
                return Ok(());
            }
            let catalog = self.inner.borrow().catalogs[&key].clone();
            let localize = |text: &str| catalog.get(text).cloned().unwrap_or_else(|| text.to_owned());
            let mut inner = self.inner.borrow_mut();
            inner.page = 0;
            inner.prepared_title = localize(&title);
            inner.choices = choices
                .iter()
                .map(|choice| GamePromptChoice {
                    label: localize(&choice.label),
                    impulse: choice.impulse,
                })
                .collect();
            inner.prepared_language = language;
            if let Some(pending) = inner.pending.as_mut() {
                pending.prepared = true;
            }
        }
        if self.inner.borrow().pending.is_some() && focus() == SeatInputFocus::Game {
            self.controller.open_prompt();
        }
        if self.inner.borrow().pending.is_none() && self.controller.prompt_active() {
            self.controller.close_prompt();
        }
        Ok(())
    }

    /// Handle one seat input event; returns whether it was consumed.
    pub fn input(&mut self, event: &SeatInputEvent) -> bool {
        if event.seat != self.inner.borrow().seat || !self.controller.prompt_active() {
            return false;
        }
        if let SeatInputEventKind::Key { code, down, repeat } = event.kind {
            if (49..=57).contains(&code) {
                if down && !repeat {
                    self.inner.borrow_mut().choose((code - 49) as usize);
                    if self.inner.borrow().pending.is_none() && self.controller.prompt_active() {
                        self.controller.close_prompt();
                    }
                }
                return true;
            }
        }
        false
    }

    /// Clear the prompt and close its menu.
    pub fn clear(&mut self) {
        self.inner.borrow_mut().clear_state();
        if self.controller.prompt_active() {
            self.controller.close_prompt();
        }
    }

    /// Clear, unregister, and drop catalog caches.
    pub fn close(mut self) {
        self.inner.borrow_mut().clear_state();
        if self.controller.prompt_active() {
            self.controller.close_prompt();
        }
        self.controller.unregister_prompt();
        self.inner.borrow_mut().catalogs.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeController {
        factory: Option<Rc<dyn Fn() -> UiMenu>>,
        active: bool,
        opens: usize,
    }

    impl GamePromptController for FakeController {
        fn register_prompt(&mut self, factory: Rc<dyn Fn() -> UiMenu>) {
            self.factory = Some(factory);
        }

        fn open_prompt(&mut self) {
            self.active = true;
            self.opens += 1;
        }

        fn close_prompt(&mut self) {
            self.active = false;
        }

        fn prompt_active(&self) -> bool {
            self.active
        }

        fn unregister_prompt(&mut self) {
            self.factory = None;
        }
    }

    struct FakeCatalogs {
        loads: Vec<String>,
    }

    impl GamePromptCatalogs for FakeCatalogs {
        fn load_catalog(
            &mut self,
            content: &ContentId,
            language: &str,
        ) -> Result<HashMap<String, String>, GamePromptError> {
            self.loads.push(format!("{}:{language}", content.as_str()));
            Ok(HashMap::from([("Pick".to_owned(), "Elegir".to_owned())]))
        }
    }

    fn identities() -> (SeatId, ActorId) {
        let owner = qa_core::identity::IdentityOwner::create("prompt").expect("owner");
        (owner.seat(0), owner.actor(0, 0))
    }

    #[test]
    fn prompts_open_localize_and_choose_by_key() {
        let (seat, me) = identities();
        let fired = Rc::new(RefCell::new(Vec::new()));
        let fired_inner = Rc::clone(&fired);
        let mut prompt = SeatGamePrompt::new(
            seat.clone(),
            Box::new(move || me.clone()),
            FakeController {
                factory: None,
                active: false,
                opens: 0,
            },
            Box::new(move |impulse| fired_inner.borrow_mut().push(impulse)),
            Box::new(|| "spanish".to_owned()),
        );
        let my_actor = (prompt.inner.borrow().actor)();
        prompt.receive(&[GamePromptEvent::Prompt {
            actor: my_actor.clone(),
            content: ContentId("q1".to_owned()),
            title: "Pick".to_owned(),
            choices: vec![
                GamePromptChoice {
                    label: "One".to_owned(),
                    impulse: 3,
                },
                GamePromptChoice {
                    label: "Two".to_owned(),
                    impulse: 4,
                },
            ],
        }]);
        let mut catalogs = FakeCatalogs { loads: Vec::new() };
        prompt
            .prepare(&mut catalogs, &|| SeatInputFocus::Game)
            .expect("prepare");
        assert!(prompt.controller.prompt_active());
        assert_eq!(prompt.inner.borrow().prepared_title, "Elegir");
        let menu = prompt.controller.factory.as_ref().expect("factory")();
        assert_eq!(menu.controls.len(), 2);
        assert_eq!(menu.controls[0].label, "1. One");
        assert!(prompt.input(&SeatInputEvent {
            seat: seat.clone(),
            time_ms: 0,
            kind: SeatInputEventKind::Key {
                code: 50,
                down: true,
                repeat: false,
            },
        }));
        assert_eq!(*fired.borrow(), vec![4]);
        assert!(!prompt.controller.prompt_active());
    }

    #[test]
    fn paginates_beyond_four_choices_and_clears() {
        let (seat, me) = identities();
        let mut prompt = SeatGamePrompt::new(
            seat.clone(),
            Box::new(move || me.clone()),
            FakeController {
                factory: None,
                active: false,
                opens: 0,
            },
            Box::new(|_| {}),
            Box::new(|| "english".to_owned()),
        );
        let my_actor = (prompt.inner.borrow().actor)();
        let choices = (0..6)
            .map(|index| GamePromptChoice {
                label: format!("c{index}"),
                impulse: index,
            })
            .collect();
        prompt.receive(&[GamePromptEvent::Prompt {
            actor: my_actor.clone(),
            content: ContentId("q1".to_owned()),
            title: "Pick".to_owned(),
            choices,
        }]);
        let mut catalogs = FakeCatalogs { loads: Vec::new() };
        prompt
            .prepare(&mut catalogs, &|| SeatInputFocus::Game)
            .expect("prepare");
        let menu = prompt.controller.factory.as_ref().expect("factory")();
        assert_eq!(menu.controls.len(), 6);
        assert_eq!(menu.controls[5].label, "Next (1/2)");
        prompt.receive(&[GamePromptEvent::ClearPrompt { actor: my_actor }]);
        assert!(!prompt.controller.prompt_active());
        prompt
            .prepare(&mut catalogs, &|| SeatInputFocus::Game)
            .expect("prepare");
        assert!(!prompt.controller.prompt_active());
    }
}
