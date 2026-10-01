//! Q2 CTF shared types (`src/content/q2/multiplayer/ctf/types.ts`).
//!
//! Original Quake II CTF 1.09b g_ctf.c/g_ctf.h. GPL-2.0-or-later.
//!
//! The donor's `Q2CtfContext` becomes the [`CtfRuntime`](super::CtfRuntime)
//! arena slot plus [`Q2CtfHooks`]; helpers borrow the arena per call instead
//! of holding the shared reference. The donor's weapon-system hooks become
//! direct arena calls (`game.weapons` plus the foundation weapon functions).

use std::collections::HashMap;

use qa_core::identity::{ActorId, SavedActorId};
use qa_core::math::Vec3;

use crate::contract::ItemId;
use crate::q2::base::player::types::Q2PlayerState;
use crate::q2::foundation::host::{Q2GameServices, Q2PresentationEvent, Q2PrintLevel};
use crate::q2::foundation::items::Q2ItemModule;

use super::match_::Q2CtfAdminSettings;

/// CTF team (`Q2CtfTeam`): 0 spectates, 1 is red, 2 is blue.
pub type Q2CtfTeam = u8;

/// Playing CTF team (`Q2CtfPlayingTeam`): 1 is red, 2 is blue.
pub type Q2CtfPlayingTeam = u8;

/// CTF match phase (`Q2CtfMatchPhase`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q2CtfMatchPhase {
    /// No match.
    #[default]
    None,
    /// Setup.
    Setup,
    /// Pregame.
    Pregame,
    /// Live game.
    Game,
    /// Post game.
    Post,
}

/// CTF tech (`Q2CtfTech`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CtfTech {
    /// Disruptor shield.
    Tech1,
    /// Power amplifier.
    Tech2,
    /// Time accel.
    Tech3,
    /// Autodoc.
    Tech4,
}

impl Q2CtfTech {
    /// Item classname.
    pub fn classname(self) -> &'static str {
        match self {
            Q2CtfTech::Tech1 => "item_tech1",
            Q2CtfTech::Tech2 => "item_tech2",
            Q2CtfTech::Tech3 => "item_tech3",
            Q2CtfTech::Tech4 => "item_tech4",
        }
    }

    /// Parse an item classname.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "item_tech1" => Some(Q2CtfTech::Tech1),
            "item_tech2" => Some(Q2CtfTech::Tech2),
            "item_tech3" => Some(Q2CtfTech::Tech3),
            "item_tech4" => Some(Q2CtfTech::Tech4),
            _ => None,
        }
    }
}

/// CTF menu action (`Q2CtfMenuAction`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CtfMenuAction {
    /// Join red.
    JoinRed,
    /// Join blue.
    JoinBlue,
    /// Observer.
    Observer,
    /// Chase camera.
    Chase,
    /// Credits.
    Credits,
    /// Request match.
    Match,
    /// Ready.
    Ready,
    /// Not ready.
    NotReady,
    /// Admin settings.
    AdminSettings,
    /// Admin start.
    AdminStart,
    /// Admin cancel.
    AdminCancel,
    /// Close.
    Close,
}

impl Q2CtfMenuAction {
    /// Command word.
    pub fn command(self) -> &'static str {
        match self {
            Q2CtfMenuAction::JoinRed => "join-red",
            Q2CtfMenuAction::JoinBlue => "join-blue",
            Q2CtfMenuAction::Observer => "observer",
            Q2CtfMenuAction::Chase => "chase",
            Q2CtfMenuAction::Credits => "credits",
            Q2CtfMenuAction::Match => "match",
            Q2CtfMenuAction::Ready => "ready",
            Q2CtfMenuAction::NotReady => "notready",
            Q2CtfMenuAction::AdminSettings => "admin-settings",
            Q2CtfMenuAction::AdminStart => "admin-start",
            Q2CtfMenuAction::AdminCancel => "admin-cancel",
            Q2CtfMenuAction::Close => "close",
        }
    }

    /// Parse a command word.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "join-red" => Some(Q2CtfMenuAction::JoinRed),
            "join-blue" => Some(Q2CtfMenuAction::JoinBlue),
            "observer" => Some(Q2CtfMenuAction::Observer),
            "chase" => Some(Q2CtfMenuAction::Chase),
            "credits" => Some(Q2CtfMenuAction::Credits),
            "match" => Some(Q2CtfMenuAction::Match),
            "ready" => Some(Q2CtfMenuAction::Ready),
            "notready" => Some(Q2CtfMenuAction::NotReady),
            "admin-settings" => Some(Q2CtfMenuAction::AdminSettings),
            "admin-start" => Some(Q2CtfMenuAction::AdminStart),
            "admin-cancel" => Some(Q2CtfMenuAction::AdminCancel),
            "close" => Some(Q2CtfMenuAction::Close),
            _ => None,
        }
    }
}

/// CTF scoreboard row (`Q2CtfScoreRow`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2CtfScoreRow {
    /// Slot.
    pub slot: i32,
    /// Name.
    pub name: String,
    /// Score.
    pub score: i32,
    /// Ping.
    pub ping: i32,
    /// Carried flag.
    pub carried_flag: Option<Q2CtfPlayingTeam>,
}

/// CTF flag state (`state` result).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CtfFlagState {
    /// At base.
    Base,
    /// Dropped.
    Dropped,
    /// Taken.
    Taken,
}

/// CTF menu entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q2CtfMenuEntry {
    /// Label.
    pub label: String,
    /// Action.
    pub action: Option<Q2CtfMenuAction>,
}

/// CTF presentation event (`Q2CtfEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q2CtfEvent {
    /// Scoreboard.
    Scoreboard {
        /// Viewer.
        actor: ActorId,
        /// Red rows.
        red: Vec<Q2CtfScoreRow>,
        /// Blue rows.
        blue: Vec<Q2CtfScoreRow>,
        /// Spectator rows.
        spectators: Vec<Q2CtfScoreRow>,
        /// Captures by team.
        captures: [i32; 2],
        /// Totals by team.
        totals: [i32; 2],
        /// Layout program.
        layout: String,
    },
    /// Hud.
    Hud {
        /// Viewer.
        actor: ActorId,
        /// Captures by team.
        captures: [i32; 2],
        /// Flag states by team.
        flag_states: [Q2CtfFlagState; 2],
        /// Viewer team.
        team: Q2CtfTeam,
        /// Carried flag.
        carried_flag: Option<Q2CtfPlayingTeam>,
        /// Held tech.
        tech: Option<Q2CtfTech>,
        /// Identified target.
        id_target: Option<ActorId>,
        /// Blinking team.
        blink_team: Option<Q2CtfPlayingTeam>,
        /// Match status text.
        match_text: String,
    },
    /// Menu.
    Menu {
        /// Viewer.
        actor: ActorId,
        /// Title.
        title: String,
        /// Entries.
        entries: Vec<Q2CtfMenuEntry>,
    },
    /// Match status.
    MatchStatus {
        /// Text.
        text: String,
    },
    /// Admin settings.
    AdminSettings {
        /// Viewer.
        actor: ActorId,
        /// Settings.
        settings: Q2CtfAdminSettings,
    },
    /// Grapple cable.
    GrappleCable {
        /// Owner.
        actor: ActorId,
        /// Cable start.
        start: Vec3,
        /// Cable end.
        end: Vec3,
        /// Start offset.
        offset: Vec3,
    },
}

/// Forced join (`Q2CtfRules["forceJoin"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Q2CtfForceJoin {
    /// Any team.
    #[default]
    Any,
    /// Red only.
    Red,
    /// Blue only.
    Blue,
}

impl Q2CtfForceJoin {
    /// Setting word.
    pub fn word(self) -> &'static str {
        match self {
            Q2CtfForceJoin::Any => "",
            Q2CtfForceJoin::Red => "red",
            Q2CtfForceJoin::Blue => "blue",
        }
    }

    /// Parse a setting word.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "" => Some(Q2CtfForceJoin::Any),
            "red" => Some(Q2CtfForceJoin::Red),
            "blue" => Some(Q2CtfForceJoin::Blue),
            _ => None,
        }
    }
}

/// CTF rules (`Q2CtfRules`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfRules {
    /// Forced join.
    pub force_join: Q2CtfForceJoin,
    /// Competition level.
    pub competition: i32,
    /// Match lock.
    pub match_lock: bool,
    /// Election percentage.
    pub election_percentage: f64,
    /// Match minutes.
    pub match_minutes: f64,
    /// Setup minutes.
    pub setup_minutes: f64,
    /// Start seconds.
    pub start_seconds: f64,
    /// Admin password.
    pub admin_password: String,
    /// Warp list.
    pub warp_list: Vec<String>,
    /// Capture limit.
    pub capture_limit: i32,
    /// Instant weapons.
    pub instant_weapons: bool,
}

/// Create CTF rules (`createQ2CtfRules`).
pub fn create_q2_ctf_rules() -> Q2CtfRules {
    Q2CtfRules {
        force_join: Q2CtfForceJoin::Any,
        competition: 0,
        match_lock: true,
        election_percentage: 66.0,
        match_minutes: 20.0,
        setup_minutes: 10.0,
        start_seconds: 20.0,
        admin_password: String::new(),
        warp_list: ["q2ctf1", "q2ctf2", "q2ctf3", "q2ctf4", "q2ctf5"]
            .iter()
            .map(|map| map.to_string())
            .collect(),
        capture_limit: 0,
        instant_weapons: false,
    }
}

impl Default for Q2CtfRules {
    fn default() -> Self {
        create_q2_ctf_rules()
    }
}

/// Match-private CTF player state (`Q2CtfPlayerState`).
///
/// Health, armor, inventory and score stay shared.
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfPlayerState {
    /// Team.
    pub team: Q2CtfTeam,
    /// Spawn state.
    pub spawn_state: i32,
    /// Last hurt-carrier time.
    pub last_hurt_carrier: Option<f64>,
    /// Last returned-flag time.
    pub last_returned_flag: Option<f64>,
    /// Last fragged-carrier time.
    pub last_fragged_carrier: Option<f64>,
    /// Flag-since time.
    pub flag_since: f64,
    /// Voted.
    pub voted: bool,
    /// Ready.
    pub ready: bool,
    /// Admin.
    pub admin: bool,
    /// Identify view.
    pub id_view: bool,
    /// Ghost code.
    pub ghost_code: Option<i32>,
    /// Regeneration time.
    pub regen_time: f64,
    /// Tech sound time.
    pub tech_sound_time: f64,
    /// Last tech message time.
    pub last_tech_message: f64,
    /// Match respawn time.
    pub match_respawn_at: Option<f64>,
}

impl Default for Q2CtfPlayerState {
    fn default() -> Self {
        Self {
            team: 0,
            spawn_state: 0,
            last_hurt_carrier: None,
            last_returned_flag: None,
            last_fragged_carrier: None,
            flag_since: 0.0,
            voted: false,
            ready: false,
            admin: false,
            id_view: true,
            ghost_code: None,
            regen_time: 0.0,
            tech_sound_time: 0.0,
            last_tech_message: 0.0,
            match_respawn_at: None,
        }
    }
}

/// CTF ghost (`Q2CtfGhost`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfGhost {
    /// Code.
    pub code: i32,
    /// Team.
    pub team: Q2CtfPlayingTeam,
    /// Name.
    pub name: String,
    /// Actor.
    pub actor: Option<ActorId>,
    /// Score.
    pub score: i32,
    /// Deaths.
    pub deaths: i32,
    /// Kills.
    pub kills: i32,
    /// Captures.
    pub captures: i32,
    /// Base defense.
    pub base_defense: i32,
    /// Carrier defense.
    pub carrier_defense: i32,
}

/// CTF election kind (`Q2CtfElection["kind"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CtfElectionKind {
    /// Match.
    Match,
    /// Admin.
    Admin,
    /// Map.
    Map,
}

/// CTF election (`Q2CtfElection`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfElection {
    /// Kind.
    pub kind: Q2CtfElectionKind,
    /// Target.
    pub target: ActorId,
    /// Map.
    pub map: String,
    /// Message.
    pub message: String,
    /// Votes.
    pub votes: i32,
    /// Needed.
    pub needed: i32,
    /// Expiry.
    pub expires: f64,
}

/// CTF match state (`Q2CtfMatchState`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q2CtfMatchState {
    /// Red captures.
    pub team1: i32,
    /// Blue captures.
    pub team2: i32,
    /// Red total.
    pub total1: i32,
    /// Blue total.
    pub total2: i32,
    /// Last flag capture time.
    pub last_flag_capture: Option<f64>,
    /// Last capture team.
    pub last_capture_team: Option<Q2CtfPlayingTeam>,
    /// Phase.
    pub phase: Q2CtfMatchPhase,
    /// Match time.
    pub match_time: f64,
    /// Last reported time.
    pub last_time: f64,
    /// Election.
    pub election: Option<Q2CtfElection>,
    /// Ghosts by code.
    pub ghosts: HashMap<i32, Q2CtfGhost>,
}

impl Q2CtfMatchState {
    /// Create match state (`new Q2CtfMatchState`).
    pub fn new() -> Self {
        Self {
            last_time: -1.0,
            ..Self::default()
        }
    }
}

/// CTF hooks (`Q2CtfHooks`).
///
/// Composition supplies existing player lifecycle and session presentation
/// services. The donor's weapon-system member becomes direct arena calls.
#[derive(Debug, Clone, Copy)]
pub struct Q2CtfHooks {
    /// Item module.
    pub items: Q2ItemModule,
    /// Read the shared player state.
    pub player: for<'a> fn(ActorId, &'a mut Q2GameServices) -> Option<&'a mut Q2PlayerState>,
    /// Set the player skin.
    pub set_skin: fn(ActorId, &mut Q2GameServices, String),
    /// Spawn the player.
    pub spawn_player: fn(ActorId, &mut Q2GameServices),
    /// Move the player to an observer.
    pub observer: fn(ActorId, &mut Q2GameServices),
    /// Teleport the player.
    pub teleport: fn(ActorId, &mut Q2GameServices, Vec3, Vec3, Vec3),
    /// Chase.
    pub chase: fn(ActorId, &mut Q2GameServices),
    /// Suppress or restore grapple prediction.
    pub set_grapple_prediction: fn(ActorId, &mut Q2GameServices, bool),
    /// Read gravity.
    pub gravity: fn(&Q2GameServices) -> f64,
    /// Emit a CTF event.
    pub emit: fn(&mut Q2GameServices, Q2CtfEvent),
    /// End the level.
    pub end_level: fn(&mut Q2GameServices, Option<String>),
    /// Kick the actor.
    pub kick: fn(ActorId, &mut Q2GameServices),
    /// Set deathmatch flags.
    pub set_deathmatch_flags: fn(&mut Q2GameServices, i32),
    /// Whether chat is allowed.
    pub chat_allowed: fn(ActorId, &mut Q2GameServices) -> bool,
}

/// CTF flag info (`CTF_FLAGS` entry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q2CtfFlagInfo {
    /// Classname.
    pub classname: &'static str,
    /// Item id.
    pub item: &'static str,
    /// Model.
    pub model: &'static str,
    /// Icon.
    pub icon: &'static str,
    /// Effect.
    pub effect: i64,
}

/// CTF flags by playing team (`CTF_FLAGS`).
pub const CTF_FLAGS: [Q2CtfFlagInfo; 2] = [
    Q2CtfFlagInfo {
        classname: "item_flag_team1",
        item: "q2:item_flag_team1",
        model: "players/male/flag1.md2",
        icon: "i_ctf1",
        effect: 0x40000,
    },
    Q2CtfFlagInfo {
        classname: "item_flag_team2",
        item: "q2:item_flag_team2",
        model: "players/male/flag2.md2",
        icon: "i_ctf2",
        effect: 0x80000,
    },
];

/// Read flag info for a playing team.
pub fn ctf_flag(team: Q2CtfPlayingTeam) -> &'static Q2CtfFlagInfo {
    match team {
        1 => &CTF_FLAGS[0],
        2 => &CTF_FLAGS[1],
        _ => panic!("CTF team is not a playing team"),
    }
}

/// Other CTF team (`otherCtfTeam`).
pub fn other_ctf_team(team: Q2CtfPlayingTeam) -> Q2CtfPlayingTeam {
    if team == 1 {
        2
    } else {
        1
    }
}

/// CTF team name (`ctfTeamName`).
pub fn ctf_team_name(team: Q2CtfTeam) -> &'static str {
    if team == 1 {
        "RED"
    } else if team == 2 {
        "BLUE"
    } else {
        "UNKNOWN"
    }
}

/// Read the admitted CTF player state (`ctfPlayer`).
pub fn ctf_player<'a>(game: &'a mut Q2GameServices, actor: &ActorId) -> &'a mut Q2CtfPlayerState {
    game.ctf
        .states
        .get_mut(actor)
        .unwrap_or_else(|| panic!("CTF player has not been admitted to the shared match"))
}

/// CTF print level.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2CtfPrintLevel {
    /// High.
    High,
    /// Medium.
    Medium,
    /// Chat.
    Chat,
}

impl From<Q2CtfPrintLevel> for Q2PrintLevel {
    fn from(level: Q2CtfPrintLevel) -> Self {
        match level {
            Q2CtfPrintLevel::High => Q2PrintLevel::High,
            Q2CtfPrintLevel::Medium => Q2PrintLevel::Medium,
            Q2CtfPrintLevel::Chat => Q2PrintLevel::Chat,
        }
    }
}

/// Print CTF text (`ctfPrint`).
pub fn ctf_print(game: &mut Q2GameServices, text: &str, actor: Option<ActorId>, level: Q2CtfPrintLevel) {
    game.host_emit(Q2PresentationEvent::Print {
        actor,
        level: level.into(),
        text: text.to_string(),
    });
}

/// CTF player name (`ctfName`).
pub fn ctf_name(game: &mut Q2GameServices, actor: &ActorId) -> String {
    let hooks = super::ctf_hooks(game);
    (hooks.player)(actor.clone(), game)
        .map(|player| player.name.clone())
        .unwrap_or_else(|| "player".to_string())
}

/// Add CTF score (`ctfScore`).
pub fn ctf_score(game: &mut Q2GameServices, actor: &ActorId, amount: i32) {
    let hooks = super::ctf_hooks(game);
    let Some(player) = (hooks.player)(actor.clone(), game) else {
        panic!("CTF score requires the shared player state");
    };
    player.score += amount;
}

/// Save a CTF actor (`saveCtfActor`).
pub fn save_ctf_actor(actor: &ActorId) -> SavedActorId {
    SavedActorId::from(actor)
}

/// Read the carried flag (`ctfCarriedFlag`).
pub fn ctf_carried_flag(game: &mut Q2GameServices, actor: &ActorId) -> Option<Q2CtfPlayingTeam> {
    let one = CTF_FLAGS[0].item.to_string();
    if game.host.inventory().count(actor, &one) > 0.0 {
        return Some(1);
    }
    let two = CTF_FLAGS[1].item.to_string();
    if game.host.inventory().count(actor, &two) > 0.0 {
        return Some(2);
    }
    None
}

/// Item id for a static path.
pub fn item_id(path: &str) -> ItemId {
    path.to_string()
}
