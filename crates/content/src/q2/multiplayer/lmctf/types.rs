//! Q2 LMCTF shared types (`src/content/q2/multiplayer/lmctf/types.ts`).
//!
//! LM_CTF 5.2/6.0 g_local.h, g_ctffunc.h and q_shared.h. GPL-2.0-or-later.
//!
//! The donor's `LmctfContext` becomes the [`LmctfRuntime`](super::LmctfRuntime)
//! arena slot plus [`LmctfHooks`]; helpers borrow the arena per call instead
//! of holding the shared reference. The donor's weapon-system hooks become
//! direct arena calls (`game.weapons` plus the foundation weapon functions).

use std::collections::HashMap;

use qa_core::identity::ActorId;
use qa_core::math::{Vec3, vec3};

use crate::q2::base::player::types::Q2PlayerState;
use crate::q2::foundation::host::{Q2GameServices, Q2Mode, Q2PresentationEvent, Q2PrintLevel, Q2TraceRequest};
use crate::q2::foundation::items::Q2ItemModule;

/// LMCTF team (`LmctfTeam`): 0 spectates, 1 is red, 2 is blue.
pub type LmctfTeam = u8;

/// Playing LMCTF team (`LmctfPlayingTeam`): 1 is red, 2 is blue.
pub type LmctfPlayingTeam = u8;

/// LMCTF rune (`LmctfRune`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LmctfRune {
    /// Damage.
    Damage,
    /// Resist.
    Resist,
    /// Haste.
    Haste,
    /// Regen.
    Regen,
    /// Vampire.
    Vampire,
}

/// LMCTF rune definition (`LmctfRuneDefinition`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LmctfRuneDefinition {
    /// Kind.
    pub kind: LmctfRune,
    /// Bit.
    pub bit: i32,
    /// Classname.
    pub classname: &'static str,
    /// Item id.
    pub item: &'static str,
    /// Model.
    pub model: &'static str,
    /// Icon.
    pub icon: &'static str,
    /// Display name.
    pub name: &'static str,
}

/// LMCTF runes (`LMCTF_RUNES`).
pub const LMCTF_RUNES: [LmctfRuneDefinition; 5] = [
    LmctfRuneDefinition {
        kind: LmctfRune::Damage,
        bit: 1,
        classname: "damage_rune",
        item: "q2:damage_rune",
        model: "models/ctf/damage/tris.md2",
        icon: "a_strength",
        name: "Damage Artifact",
    },
    LmctfRuneDefinition {
        kind: LmctfRune::Haste,
        bit: 4,
        classname: "haste_rune",
        item: "q2:haste_rune",
        model: "models/ctf/haste/tris.md2",
        icon: "a_haste",
        name: "Haste Artifact",
    },
    LmctfRuneDefinition {
        kind: LmctfRune::Resist,
        bit: 2,
        classname: "resist_rune",
        item: "q2:resist_rune",
        model: "models/ctf/resist/tris.md2",
        icon: "a_resist",
        name: "Resist Artifact",
    },
    LmctfRuneDefinition {
        kind: LmctfRune::Regen,
        bit: 8,
        classname: "regen_rune",
        item: "q2:regen_rune",
        model: "models/ctf/regen/tris.md2",
        icon: "a_regen",
        name: "Regen Artifact",
    },
    LmctfRuneDefinition {
        kind: LmctfRune::Vampire,
        bit: 16,
        classname: "vampire_rune",
        item: "q2:vampire_rune",
        model: "models/ctf/resist/tris.md2",
        icon: "k_redkey",
        name: "Vampire Artifact",
    },
];

/// LMCTF rules (`LmctfRules`).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfRules {
    /// CTF flags.
    pub ctf_flags: i32,
    /// Referee flags.
    pub ref_flags: i32,
    /// Rune bits.
    pub runes: i32,
    /// Skin set.
    pub skin_set: i32,
    /// Flag init.
    pub flag_init: bool,
    /// Disabled weapons.
    pub disabled_weapons: i32,
    /// Time limit minutes.
    pub time_limit_minutes: f64,
    /// Frag limit.
    pub frag_limit: i32,
    /// Map list.
    pub map_list: Vec<String>,
    /// Rcon password.
    pub rcon_password: String,
    /// Fast switch.
    pub fast_switch: bool,
    /// Referee password.
    pub ref_password: String,
    /// Auto lock.
    pub auto_lock: bool,
    /// Countdown seconds.
    pub countdown_seconds: f64,
    /// Quad seconds.
    pub quad_seconds: f64,
}

/// Create LMCTF rules (`createLmctfRules`).
pub fn create_lmctf_rules() -> LmctfRules {
    LmctfRules {
        ctf_flags: 0,
        ref_flags: 0,
        runes: 15,
        skin_set: 0,
        flag_init: false,
        disabled_weapons: 0,
        time_limit_minutes: 0.0,
        frag_limit: 0,
        map_list: Vec::new(),
        rcon_password: String::new(),
        fast_switch: false,
        ref_password: String::new(),
        auto_lock: false,
        countdown_seconds: 15.0,
        quad_seconds: 30.0,
    }
}

impl Default for LmctfRules {
    fn default() -> Self {
        create_lmctf_rules()
    }
}

/// LMCTF scoreboard row (`LmctfScoreRow`).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfScoreRow {
    /// Actor.
    pub actor: ActorId,
    /// Slot.
    pub slot: i32,
    /// Name.
    pub name: String,
    /// Team.
    pub team: LmctfTeam,
    /// Score.
    pub score: f64,
    /// Ping.
    pub ping: i32,
}

/// LMCTF menu entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LmctfMenuEntry {
    /// Label.
    pub label: String,
    /// Command.
    pub command: Option<String>,
}

/// LMCTF presentation event (`LmctfEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum LmctfEvent {
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
    /// Menu.
    Menu {
        /// Viewer.
        actor: ActorId,
        /// Title.
        title: String,
        /// Entries.
        entries: Vec<LmctfMenuEntry>,
    },
    /// Scoreboard.
    Scoreboard {
        /// Viewer.
        actor: ActorId,
        /// Rows.
        rows: Vec<LmctfScoreRow>,
        /// Layout program.
        layout: String,
    },
    /// Hud.
    Hud {
        /// Viewer.
        actor: ActorId,
        /// Team.
        team: LmctfTeam,
        /// Carried flag.
        carried_flag: bool,
        /// Held rune.
        rune: Option<LmctfRune>,
        /// Layout program.
        layout: String,
    },
    /// Score log.
    ScoreLog {
        /// Scorer.
        actor: ActorId,
        /// Victim.
        victim: Option<ActorId>,
        /// Name.
        name: String,
        /// Amount.
        amount: i32,
        /// Seconds.
        seconds: f64,
    },
}

/// LMCTF hooks (`LmctfHooks`).
///
/// Composition supplies existing player lifecycle and session presentation
/// services. The donor's weapon-system member becomes direct arena calls.
#[derive(Debug, Clone, Copy)]
pub struct LmctfHooks {
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
    /// Emit an LMCTF event.
    pub emit: fn(&mut Q2GameServices, LmctfEvent),
    /// End the level.
    pub end_level: fn(&mut Q2GameServices, Option<String>),
    /// Kick the actor.
    pub kick: fn(ActorId, &mut Q2GameServices),
    /// Set deathmatch flags.
    pub set_deathmatch_flags: fn(&mut Q2GameServices, i32),
    /// Whether chat is allowed.
    pub chat_allowed: fn(ActorId, &mut Q2GameServices) -> bool,
}

/// Match-private LMCTF player state (`LmctfPlayerState`).
///
/// Player score, health, armor and item counts remain shared.
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfPlayerState {
    /// Plasma mode.
    pub plasma_mode: bool,
    /// Team.
    pub team: LmctfTeam,
    /// Observer team.
    pub observer_team: LmctfTeam,
    /// Held rune.
    pub rune: Option<ActorId>,
    /// Regeneration frame.
    pub regen_frame: i32,
    /// Kill-carrier time.
    pub kill_carrier_time: f64,
    /// Hit-carrier time.
    pub hit_carrier_time: f64,
    /// Return-flag time.
    pub return_flag_time: f64,
    /// Defend-flag time.
    pub defend_flag_time: f64,
    /// Extra flags.
    pub extra_flags: i32,
    /// Spawn state.
    pub spawn_state: i32,
    /// Statistics.
    pub statistics: HashMap<String, f64>,
}

impl Default for LmctfPlayerState {
    fn default() -> Self {
        Self {
            plasma_mode: false,
            team: 0,
            observer_team: 0,
            rune: None,
            regen_frame: 0,
            kill_carrier_time: 0.0,
            hit_carrier_time: 0.0,
            return_flag_time: 0.0,
            defend_flag_time: 0.0,
            extra_flags: 16 | 32,
            spawn_state: 0,
            statistics: HashMap::new(),
        }
    }
}

/// Read the admitted LMCTF player state (`lmctfPlayer`).
pub fn lmctf_player<'a>(game: &'a mut Q2GameServices, actor: &ActorId) -> &'a mut LmctfPlayerState {
    game.lmctf.states.get_mut(actor).unwrap_or_else(|| panic!("LMCTF player has not been admitted"))
}

/// LMCTF player name (`lmctfName`).
pub fn lmctf_name(game: &mut Q2GameServices, actor: &ActorId) -> String {
    let hooks = super::lmctf_hooks(game);
    (hooks.player)(actor.clone(), game).map(|player| player.name.clone()).unwrap_or_else(|| "player".to_string())
}

/// Print LMCTF text (`lmctfPrint`).
pub fn lmctf_print(game: &mut Q2GameServices, text: &str, actor: Option<ActorId>) {
    game.host_emit(Q2PresentationEvent::Print { actor, level: Q2PrintLevel::High, text: text.to_string() });
}

/// Whether the match can score (`LmctfContext::canScore`).
pub fn lmctf_can_score(game: &Q2GameServices) -> bool {
    super::match_::LmctfMatch::can_score_state(&game.lmctf.match_state)
}

/// Whether flags are touchable (`LmctfContext::flagsTouchable`).
pub fn lmctf_flags_touchable(game: &Q2GameServices) -> bool {
    game.lmctf.match_state.phase != super::match_::LmctfMatchPhase::Countdown
}

/// Add an LMCTF statistic (`lmctfStat`).
pub fn lmctf_stat(game: &mut Q2GameServices, actor: &ActorId, name: &str, amount: i32) {
    if !lmctf_can_score(game) {
        return;
    }
    let state = lmctf_player(game, actor);
    state.statistics.insert(name.to_string(), state.statistics.get(name).copied().unwrap_or(0.0) + f64::from(amount));
}

/// Add LMCTF score (`lmctfScore`).
pub fn lmctf_score(game: &mut Q2GameServices, actor: &ActorId, amount: i32, name: &str, victim: Option<ActorId>) {
    let hooks = super::lmctf_hooks(game);
    let Some(player) = (hooks.player)(actor.clone(), game) else {
        panic!("LMCTF score requires the shared player state");
    };
    player.score += amount;
    lmctf_stat(game, actor, "score", amount);
    (hooks.emit)(
        game,
        LmctfEvent::ScoreLog { actor: actor.clone(), victim, name: name.to_string(), amount, seconds: game.now() },
    );
}

/// Whether the actor is an active player (`lmctfActive`).
pub fn lmctf_active(game: &mut Q2GameServices, actor: &ActorId) -> bool {
    let member = game.lmctf.states.get(actor).map(|state| state.team);
    let Some(team) = member else {
        return false;
    };
    let hooks = super::lmctf_hooks(game);
    let Some(player) = (hooks.player)(actor.clone(), game) else {
        return false;
    };
    let (connected, spectator) = (player.connected, player.spectator);
    if !connected || spectator {
        return false;
    }
    if team == 0 && matches!(game.options.mode, Q2Mode::Deathmatch) {
        return false;
    }
    game.entity(actor).is_some()
}

/// Toss an entity forward (`lmctfToss`).
pub fn lmctf_toss(entity: ActorId, player: ActorId, game: &mut Q2GameServices, forward: Vec3) {
    let body = game.body_of(player.clone());
    let end = vec3(body.origin.x + forward.x * 24.0, body.origin.y + forward.y * 24.0, body.origin.z + forward.z * 24.0 - 16.0);
    let bounds = game.body_of(entity.clone()).bounds;
    let origin = game
        .host
        .trace(&Q2TraceRequest { start: body.origin, end, bounds: Some(bounds), ignore: Some(player), mask: 1, exclude: Vec::new() })
        .end;
    let mut moved = game.body_of(entity.clone());
    moved.origin = origin;
    moved.velocity = vec3(forward.x * 200.0, forward.y * 200.0, 300.0);
    game.write_body(entity, &moved, true);
}

/// LMCTF map change (`LmctfMapChange`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LmctfMapChange {
    /// Map.
    pub map: String,
    /// Countdown.
    pub countdown: bool,
}

/// LMCTF travel player (`LmctfTravel["players"]` entry).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LmctfTravelPlayer {
    /// Slot.
    pub slot: i32,
    /// Team.
    pub team: LmctfTeam,
    /// Observer team.
    pub observer_team: LmctfTeam,
    /// Extra flags.
    pub extra_flags: i32,
}

/// LMCTF travel (`LmctfTravel`).
#[derive(Debug, Clone, PartialEq)]
pub struct LmctfTravel {
    /// Rules.
    pub rules: LmctfRules,
    /// Countdown.
    pub countdown: bool,
    /// Paused.
    pub paused: bool,
    /// Players.
    pub players: Vec<LmctfTravelPlayer>,
}
