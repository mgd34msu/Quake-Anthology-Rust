//! Quake II match menus, prompts, and scoreboards for one seat.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/q2-match-ui.ts`
//! (`Q2MatchUi`). Menus use the ported [`UiMenu`]/[`UiControl`]/[`menu_row`] builders and
//! prompts use the ported [`HudPrompt`]; the composition events
//! (`src/content/composition/q2/types.ts` plus the CTF/LMCTF match types, all out of
//! scope) are shimmed to the fields this class reads. The controller arrives through the
//! [`Q2MatchUiController`] seam because the merged native UI controller exposes no
//! public open API yet; when it does, the seam implementation is one line per method.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use qa_client::ui::common::layout::{menu_row, MenuRowOptions};
use qa_client::ui::hud::HudPrompt;
use qa_client::ui::types::{UiControl, UiControlId, UiControlKind, UiMenu, UiMenuId};
use qa_core::identity::ActorId;

/// Match menu identity (donor `menu:application:match`).
const MATCH_MENU_ID: &str = "menu:application:match";

/// Menu controller seam (donor native-controller surface this class uses).
pub trait Q2MatchUiController {
    /// Close every menu.
    fn close_all(&mut self);
    /// Register a menu factory.
    fn register_menu(&mut self, id: UiMenuId, factory: qa_client::ui::common::controller::UiMenuFactory);
    /// Remove a menu factory.
    fn unregister_menu(&mut self, id: &UiMenuId);
    /// Open a registered menu.
    fn open_menu(&mut self, id: &UiMenuId);
}

/// One scoreboard row (donor CTF/LMCTF score rows).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MatchScoreRow {
    /// Player name.
    pub name: String,
    /// Score.
    pub score: i32,
    /// Ping in milliseconds.
    pub ping: i32,
}

/// CTF tech pickup (donor `Q2CtfTech`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q2CtfTech {
    /// Disruptor shield.
    Tech1,
    /// Power amplifier.
    Tech2,
    /// Time acceleration.
    Tech3,
    /// Autodoc.
    Tech4,
}

impl Q2CtfTech {
    /// Display name (donor inline tech map).
    #[must_use]
    pub fn display_name(self) -> &'static str {
        match self {
            Self::Tech1 => "Disruptor Shield",
            Self::Tech2 => "Power Amplifier",
            Self::Tech3 => "Time Accel",
            Self::Tech4 => "AutoDoc",
        }
    }
}

/// One menu entry command (donor CTF `action` / LMCTF `command`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Q2MatchMenuCommand {
    /// CTF action, sent as `ctf-menu <action>`.
    CtfAction(String),
    /// Raw command line, split on whitespace.
    Words(String),
}

/// One menu entry (donor match-menu entries).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2MatchMenuEntry {
    /// Entry label.
    pub label: String,
    /// Entry command, or [`None`] for a disabled row.
    pub command: Option<Q2MatchMenuCommand>,
}

/// One admin setting value (donor `Q2CtfAdminSettings` values).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2AdminSetting {
    /// Boolean toggle.
    Boolean(bool),
    /// Numeric text entry.
    Number(f64),
    /// String text entry.
    Text(String),
}

impl Q2AdminSetting {
    /// Donor `String(value)` rendering.
    fn display(&self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::Number(value) => {
                if value.fract() == 0.0 && value.abs() < 1e15 {
                    format!("{}", *value as i64)
                } else {
                    format!("{value}")
                }
            }
            Self::Text(value) => value.clone(),
        }
    }
}

/// Match event for one seat (donor CTF/LMCTF composition-event subset).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2MatchUiEvent {
    /// Grapple cable update, ignored.
    GrappleCable {
        /// Target actor.
        actor: ActorId,
    },
    /// Match status line, always printed.
    MatchStatus {
        /// Status text.
        text: String,
    },
    /// LMCTF score log line.
    ScoreLog {
        /// Target actor.
        actor: ActorId,
        /// Player name.
        name: String,
        /// Score delta.
        amount: i32,
    },
    /// CTF HUD refresh.
    HudCtf {
        /// Target actor.
        actor: ActorId,
        /// Team number (1 red, 2 blue, else spectator).
        team: u32,
        /// Red/blue captures.
        captures: [i32; 2],
        /// Red/blue flag states.
        flag_states: [String; 2],
        /// Carried tech, when held.
        tech: Option<Q2CtfTech>,
        /// Match clock text, empty when unset.
        match_text: String,
        /// Aimed player, when identified.
        id_target: Option<ActorId>,
        /// Blinking team, when a capture just happened.
        blink_team: Option<u32>,
        /// Whether the seat carries the flag.
        carried_flag: bool,
    },
    /// LMCTF HUD refresh.
    HudLmctf {
        /// Target actor.
        actor: ActorId,
        /// Team number (1 red, 2 blue, else spectator).
        team: u32,
        /// Carried rune, when held.
        rune: Option<String>,
        /// Whether the seat carries the flag.
        carried_flag: bool,
    },
    /// CTF scoreboard.
    ScoreboardCtf {
        /// Target actor.
        actor: ActorId,
        /// Red rows.
        red: Vec<Q2MatchScoreRow>,
        /// Blue rows.
        blue: Vec<Q2MatchScoreRow>,
        /// Spectator rows.
        spectators: Vec<Q2MatchScoreRow>,
        /// Red/blue captures.
        captures: [i32; 2],
    },
    /// LMCTF scoreboard.
    ScoreboardLmctf {
        /// Target actor.
        actor: ActorId,
        /// Score rows.
        rows: Vec<Q2MatchScoreRow>,
    },
    /// Match menu.
    Menu {
        /// Target actor.
        actor: ActorId,
        /// Menu title.
        title: String,
        /// Menu entries.
        entries: Vec<Q2MatchMenuEntry>,
    },
    /// CTF admin settings.
    AdminSettings {
        /// Target actor.
        actor: ActorId,
        /// Settings in donor order.
        settings: Vec<(String, Q2AdminSetting)>,
    },
}

impl Q2MatchUiEvent {
    /// Target actor, when the event is addressed.
    fn actor(&self) -> Option<&ActorId> {
        match self {
            Self::MatchStatus { .. } => None,
            Self::GrappleCable { actor }
            | Self::ScoreLog { actor, .. }
            | Self::HudCtf { actor, .. }
            | Self::HudLmctf { actor, .. }
            | Self::ScoreboardCtf { actor, .. }
            | Self::ScoreboardLmctf { actor, .. }
            | Self::Menu { actor, .. }
            | Self::AdminSettings { actor, .. } => Some(actor),
        }
    }
}

/// Match UI command sink (donor `Q2MatchUi` command callback).
pub type Q2MatchUiCommand = Rc<dyn Fn(&str, &[String])>;
/// Match UI print sink (donor `Q2MatchUi` print callback).
pub type Q2MatchUiPrint = Rc<dyn Fn(&str)>;

/// Quake II match UI for one seat (donor `Q2MatchUi`).
pub struct Q2MatchUi<C> {
    actor: ActorId,
    controller: Rc<RefCell<C>>,
    command: Q2MatchUiCommand,
    print: Q2MatchUiPrint,
    names: HashMap<ActorId, String>,
    menu_open: bool,
    prompts: Vec<HudPrompt>,
}

impl<C: Q2MatchUiController + 'static> Q2MatchUi<C> {
    /// Build the match UI for one seat's menus.
    pub fn new(actor: ActorId, controller: Rc<RefCell<C>>, command: Q2MatchUiCommand, print: Q2MatchUiPrint) -> Self {
        Self {
            actor,
            controller,
            command,
            print,
            names: HashMap::new(),
            menu_open: false,
            prompts: Vec::new(),
        }
    }

    /// Current HUD prompts (donor `prompts`).
    #[must_use]
    pub fn prompts(&self) -> &[HudPrompt] {
        &self.prompts
    }

    /// Remember one player's display name for id-target lines (donor `name`).
    pub fn name(&mut self, actor: ActorId, name: String) {
        self.names.insert(actor, name);
    }

    /// Show a button menu (donor `menu`).
    fn menu(&mut self, title: String, entries: Vec<(String, Option<Vec<String>>)>) {
        self.controller.borrow_mut().close_all();
        if self.menu_open {
            self.controller.borrow_mut().unregister_menu(&Self::menu_id());
            self.menu_open = false;
        }
        let controller = Rc::clone(&self.controller);
        let command = Rc::clone(&self.command);
        let id = Self::menu_id();
        let factory: qa_client::ui::common::controller::UiMenuFactory = Rc::new(move || {
            let options = MenuRowOptions::default();
            UiMenu {
                scroll: None,
                id: Self::menu_id(),
                title: title.clone(),
                full_screen: true,
                controls: entries
                    .iter()
                    .enumerate()
                    .map(|(index, (label, words))| {
                        let controller = Rc::clone(&controller);
                        let command = Rc::clone(&command);
                        let words = words.clone();
                        UiControl {
                            id: UiControlId::new(&format!("ui:match:{index}")).expect("match row id"),
                            rect: menu_row(index as i32 + 1, &options),
                            label: label.clone(),
                            enabled: words.is_some(),
                            visible: true,
                            kind: UiControlKind::Button {
                                on_activate: Rc::new(move |_| {
                                    controller.borrow_mut().close_all();
                                    if let Some(words) = &words {
                                        if let Some((name, args)) = words.split_first() {
                                            command(name, args);
                                        }
                                    }
                                }),
                            },
                        }
                    })
                    .collect(),
                on_open: Rc::new(|_| {}),
                on_close: Rc::new(|_| {}),
            }
        });
        self.controller.borrow_mut().register_menu(id.clone(), factory);
        self.menu_open = true;
        self.controller.borrow_mut().open_menu(&id);
    }

    /// Match menu identity.
    fn menu_id() -> UiMenuId {
        UiMenuId::new(MATCH_MENU_ID).expect("match menu id")
    }

    /// Receive one match event (donor `receive`).
    pub fn receive(&mut self, event: &Q2MatchUiEvent) {
        if event.actor().is_some_and(|actor| actor != &self.actor) {
            return;
        }
        match event {
            Q2MatchUiEvent::GrappleCable { .. } => {}
            Q2MatchUiEvent::MatchStatus { text } => (self.print)(text),
            Q2MatchUiEvent::ScoreLog { name, amount, .. } => {
                (self.print)(&format!("{name}: {}{amount}\n", if *amount > 0 { "+" } else { "" }));
            }
            Q2MatchUiEvent::HudCtf {
                team,
                captures,
                flag_states,
                tech,
                match_text,
                id_target,
                blink_team,
                carried_flag,
                ..
            } => {
                let mut values = vec![team_label(*team)];
                values.push(format!("Red {} ({})", captures[0], flag_states[0]));
                values.push(format!("Blue {} ({})", captures[1], flag_states[1]));
                if let Some(tech) = tech {
                    values.push(tech.display_name().to_string());
                }
                if !match_text.is_empty() {
                    values.push(match_text.clone());
                }
                if let Some(target) = id_target {
                    if let Some(name) = self.names.get(target) {
                        values.push(name.clone());
                    }
                }
                if let Some(blink) = blink_team {
                    values.push(format!(
                        "{} captured the flag",
                        if *blink == 1 { "Red" } else { "Blue" }
                    ));
                }
                if *carried_flag {
                    values.push("Carrying flag".to_string());
                }
                self.set_prompts(values);
            }
            Q2MatchUiEvent::HudLmctf {
                team,
                rune,
                carried_flag,
                ..
            } => {
                let mut values = vec![team_label(*team)];
                if let Some(rune) = rune {
                    values.push(format!("{rune} artifact"));
                }
                if *carried_flag {
                    values.push("Carrying flag".to_string());
                }
                self.set_prompts(values);
            }
            Q2MatchUiEvent::ScoreboardCtf {
                red,
                blue,
                spectators,
                captures,
                ..
            } => {
                let rows: Vec<&Q2MatchScoreRow> = red.iter().chain(blue.iter()).chain(spectators.iter()).collect();
                self.menu(
                    format!("Red {} - Blue {}", captures[0], captures[1]),
                    Self::score_entries(&rows),
                );
            }
            Q2MatchUiEvent::ScoreboardLmctf { rows, .. } => {
                let rows: Vec<&Q2MatchScoreRow> = rows.iter().collect();
                self.menu("Scores".to_string(), Self::score_entries(&rows));
            }
            Q2MatchUiEvent::Menu { title, entries, .. } => {
                self.menu(
                    title.clone(),
                    entries
                        .iter()
                        .map(|entry| {
                            let words = entry.command.as_ref().map(|command| match command {
                                Q2MatchMenuCommand::CtfAction(action) => {
                                    vec!["ctf-menu".to_string(), action.clone()]
                                }
                                Q2MatchMenuCommand::Words(line) => {
                                    line.split_whitespace().map(ToString::to_string).collect()
                                }
                            });
                            (entry.label.clone(), words)
                        })
                        .collect(),
                );
            }
            Q2MatchUiEvent::AdminSettings { settings, .. } => self.admin_menu(settings),
        }
    }

    /// Scoreboard entries with a closing row.
    fn score_entries(rows: &[&Q2MatchScoreRow]) -> Vec<(String, Option<Vec<String>>)> {
        let mut entries: Vec<(String, Option<Vec<String>>)> = rows
            .iter()
            .map(|row| (format!("{}   {}   {} ms", row.name, row.score, row.ping), None))
            .collect();
        entries.push(("Close".to_string(), Some(vec!["score".to_string()])));
        entries
    }

    /// Show the admin settings menu (donor `admin-settings`).
    fn admin_menu(&mut self, settings: &[(String, Q2AdminSetting)]) {
        self.controller.borrow_mut().close_all();
        if self.menu_open {
            self.controller.borrow_mut().unregister_menu(&Self::menu_id());
            self.menu_open = false;
        }
        let controller = Rc::clone(&self.controller);
        let command = Rc::clone(&self.command);
        let settings = settings.to_vec();
        let id = Self::menu_id();
        let factory: qa_client::ui::common::controller::UiMenuFactory = Rc::new(move || {
            let options = MenuRowOptions::default();
            let mut controls: Vec<UiControl> = settings
                .iter()
                .enumerate()
                .map(|(index, (key, value))| {
                    let spaced = insert_spaces(key);
                    let mut label = String::with_capacity(spaced.len());
                    let mut chars = spaced.chars();
                    if let Some(first) = chars.next() {
                        label.extend(first.to_uppercase());
                        label.push_str(chars.as_str());
                    }
                    let base = (
                        UiControlId::new(&format!("ui:match:{key}")).expect("match setting id"),
                        menu_row(index as i32 + 1, &options),
                        label,
                    );
                    match value {
                        Q2AdminSetting::Boolean(checked) => {
                            let command = Rc::clone(&command);
                            let key = key.clone();
                            let checked = *checked;
                            UiControl {
                                id: base.0,
                                rect: base.1,
                                label: base.2,
                                enabled: true,
                                visible: true,
                                kind: UiControlKind::Toggle {
                                    checked,
                                    on_change: Rc::new(move |_, checked| {
                                        command("ctf-settings", &[key.clone(), checked.to_string()]);
                                    }),
                                },
                            }
                        }
                        setting => {
                            let command = Rc::clone(&command);
                            let key = key.clone();
                            UiControl {
                                id: base.0,
                                rect: base.1,
                                label: base.2,
                                enabled: true,
                                visible: true,
                                kind: UiControlKind::TextEntry {
                                    masked: false,
                                    text: setting.display(),
                                    maximum_length: 8,
                                    on_change: Rc::new(|_, _| {}),
                                    on_submit: Rc::new(move |_, text| {
                                        command("ctf-settings", &[key.clone(), text.to_string()]);
                                    }),
                                },
                            }
                        }
                    }
                })
                .collect();
            let closer = Rc::clone(&controller);
            controls.push(UiControl {
                id: UiControlId::new("ui:match:close").expect("match close id"),
                rect: menu_row(10, &options),
                label: "Close".to_string(),
                enabled: true,
                visible: true,
                kind: UiControlKind::Button {
                    on_activate: Rc::new(move |_| closer.borrow_mut().close_all()),
                },
            });
            UiMenu {
                scroll: None,
                id: Self::menu_id(),
                title: "Match settings".to_string(),
                full_screen: true,
                controls,
                on_open: Rc::new(|_| {}),
                on_close: Rc::new(|_| {}),
            }
        });
        self.controller.borrow_mut().register_menu(id.clone(), factory);
        self.menu_open = true;
        self.controller.borrow_mut().open_menu(&id);
    }

    /// Replace the HUD prompts (donor prompt mapping).
    fn set_prompts(&mut self, values: Vec<String>) {
        self.prompts = values
            .into_iter()
            .map(|action| HudPrompt {
                action,
                binding: String::new(),
                icon: None,
            })
            .collect();
    }

    /// Release the current menu (donor `close`).
    pub fn close(&mut self) {
        if self.menu_open {
            self.controller.borrow_mut().unregister_menu(&Self::menu_id());
            self.menu_open = false;
        }
    }
}

/// Team number to label (donor `hud` team mapping).
fn team_label(team: u32) -> String {
    match team {
        1 => "Red".to_string(),
        2 => "Blue".to_string(),
        _ => "Spectator".to_string(),
    }
}

/// Insert spaces before capitals (donor `key.replace(/([A-Z])/gu, " $1")`).
fn insert_spaces(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for ch in key.chars() {
        if ch.is_ascii_uppercase() {
            out.push(' ');
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use std::collections::HashMap;

    struct Stub {
        menus: HashMap<String, qa_client::ui::common::controller::UiMenuFactory>,
        opened: Vec<String>,
        closes: u32,
    }

    impl Q2MatchUiController for Stub {
        fn close_all(&mut self) {
            self.closes += 1;
        }
        fn register_menu(&mut self, id: UiMenuId, factory: qa_client::ui::common::controller::UiMenuFactory) {
            self.menus.insert(id.as_str().to_string(), factory);
        }
        fn unregister_menu(&mut self, id: &UiMenuId) {
            self.menus.remove(id.as_str());
        }
        fn open_menu(&mut self, id: &UiMenuId) {
            self.opened.push(id.as_str().to_string());
        }
    }

    type Harness = (
        Q2MatchUi<Stub>,
        Rc<RefCell<Stub>>,
        Rc<RefCell<Vec<String>>>,
        Rc<RefCell<Vec<String>>>,
        IdentityOwner,
    );

    fn harness() -> Harness {
        let owner = IdentityOwner::create("q2-match-ui").unwrap();
        let actor = owner.actor(0, 1);
        let controller = Rc::new(RefCell::new(Stub {
            menus: HashMap::new(),
            opened: Vec::new(),
            closes: 0,
        }));
        let printed: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sent: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));
        let sink = Rc::clone(&printed);
        let forward = Rc::clone(&sent);
        let ui = Q2MatchUi::new(
            actor,
            Rc::clone(&controller),
            Rc::new(move |name, args| forward.borrow_mut().push(format!("{name} {}", args.join(" ")))),
            Rc::new(move |text| sink.borrow_mut().push(text.to_string())),
        );
        (ui, controller, printed, sent, owner)
    }

    #[test]
    fn foreign_actors_are_ignored() {
        let (mut ui, _, printed, _, owner) = harness();
        let foreign = owner.actor(9, 9);
        ui.receive(&Q2MatchUiEvent::MatchStatus { text: "go".to_string() });
        ui.receive(&Q2MatchUiEvent::ScoreLog {
            actor: foreign,
            name: "x".to_string(),
            amount: 1,
        });
        assert_eq!(printed.borrow().as_slice(), &["go".to_string()]);
    }

    #[test]
    fn hud_builds_prompts() {
        let (mut ui, _, _, _, owner) = harness();
        let actor = owner.actor(0, 1);
        let target = owner.actor(1, 1);
        ui.name(target.clone(), "Ranger".to_string());
        ui.receive(&Q2MatchUiEvent::HudCtf {
            actor,
            team: 1,
            captures: [3, 2],
            flag_states: ["home".to_string(), "taken".to_string()],
            tech: Some(Q2CtfTech::Tech2),
            match_text: "12:00".to_string(),
            id_target: Some(target),
            blink_team: Some(2),
            carried_flag: true,
        });
        let actions: Vec<&str> = ui.prompts().iter().map(|prompt| prompt.action.as_str()).collect();
        assert_eq!(
            actions,
            [
                "Red",
                "Red 3 (home)",
                "Blue 2 (taken)",
                "Power Amplifier",
                "12:00",
                "Ranger",
                "Blue captured the flag",
                "Carrying flag"
            ]
        );
    }

    #[test]
    fn scoreboard_opens_titled_menu() {
        let (mut ui, controller, _, _, owner) = harness();
        let actor = owner.actor(0, 1);
        ui.receive(&Q2MatchUiEvent::ScoreboardCtf {
            actor,
            red: vec![Q2MatchScoreRow {
                name: "a".to_string(),
                score: 1,
                ping: 2,
            }],
            blue: Vec::new(),
            spectators: Vec::new(),
            captures: [1, 0],
        });
        let controller = controller.borrow();
        assert_eq!(controller.opened, vec![MATCH_MENU_ID.to_string()]);
        let menu = controller.menus[MATCH_MENU_ID]();
        assert_eq!(menu.title, "Red 1 - Blue 0");
        assert_eq!(menu.controls.len(), 2);
        assert!(!menu.controls[0].enabled);
    }

    #[test]
    fn admin_menu_labels_and_toggles() {
        let (mut ui, controller, _, sent, owner) = harness();
        let actor = owner.actor(0, 1);
        ui.receive(&Q2MatchUiEvent::AdminSettings {
            actor,
            settings: vec![
                ("weaponsStay".to_string(), Q2AdminSetting::Boolean(true)),
                ("matchMinutes".to_string(), Q2AdminSetting::Number(10.0)),
            ],
        });
        let controller = controller.borrow();
        let menu = controller.menus[MATCH_MENU_ID]();
        assert_eq!(menu.controls[0].label, "Weapons Stay");
        assert_eq!(menu.controls[1].label, "Match Minutes");
        let seat = owner.seat(0);
        if let UiControlKind::Toggle { on_change, .. } = &menu.controls[0].kind {
            on_change(seat.clone(), false);
        } else {
            panic!("expected toggle");
        }
        if let UiControlKind::TextEntry { on_submit, .. } = &menu.controls[1].kind {
            on_submit(seat, "20");
        } else {
            panic!("expected text entry");
        }
        assert_eq!(
            sent.borrow().as_slice(),
            &[
                "ctf-settings weaponsStay false".to_string(),
                "ctf-settings matchMinutes 20".to_string()
            ]
        );
    }

    #[test]
    fn close_releases_menu() {
        let (mut ui, controller, _, _, owner) = harness();
        let actor = owner.actor(0, 1);
        ui.receive(&Q2MatchUiEvent::ScoreboardLmctf {
            actor,
            rows: Vec::new(),
        });
        assert!(controller.borrow().menus.contains_key(MATCH_MENU_ID));
        ui.close();
        assert!(!controller.borrow().menus.contains_key(MATCH_MENU_ID));
    }
}
