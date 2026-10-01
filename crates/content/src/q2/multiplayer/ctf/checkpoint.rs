//! Q2 CTF checkpoint (`src/content/q2/multiplayer/ctf/checkpoint.ts`).
//!
//! Checkpoints are data-only; byte framing belongs to `qa-app`.

use qa_core::identity::{ActorId, SavedActorId};

use crate::q2::equipment::grapple_services::{
    capture_ctf_grapple, restore_ctf_grapple, CtfGrappleCheckpoint, CtfGrapplePhase, CtfGrappleState,
};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices};

use super::types::{
    save_ctf_actor, Q2CtfElectionKind, Q2CtfForceJoin, Q2CtfMatchPhase, Q2CtfPlayerState, Q2CtfPlayingTeam,
};

/// CTF rules checkpoint (`Q2CtfCheckpoint["rules"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfRulesCheckpoint {
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
    /// Capture limit.
    pub capture_limit: i32,
    /// Instant weapons.
    pub instant_weapons: bool,
}

/// CTF player checkpoint entry (`Q2CtfCheckpoint["players"]` entry).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfPlayerCheckpoint {
    /// Actor.
    pub actor: SavedActorId,
    /// Match-private state.
    pub state: Q2CtfPlayerState,
    /// Grapple state.
    pub grapple: CtfGrappleCheckpoint,
}

/// CTF ghost checkpoint (`GhostCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfGhostCheckpoint {
    /// Code.
    pub code: i32,
    /// Team.
    pub team: Q2CtfPlayingTeam,
    /// Name.
    pub name: String,
    /// Actor.
    pub actor: Option<SavedActorId>,
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

/// CTF election checkpoint (`ElectionCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfElectionCheckpoint {
    /// Kind.
    pub kind: Q2CtfElectionKind,
    /// Target.
    pub target: SavedActorId,
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

/// CTF match checkpoint (`Q2CtfCheckpoint["match"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfMatchCheckpoint {
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
    pub election: Option<Q2CtfElectionCheckpoint>,
    /// Ghosts by code.
    pub ghosts: Vec<Q2CtfGhostCheckpoint>,
}

/// CTF checkpoint (`Q2CtfCheckpoint`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q2CtfCheckpoint {
    /// Version.
    pub version: u32,
    /// Rules.
    pub rules: Q2CtfRulesCheckpoint,
    /// Match.
    pub match_state: Q2CtfMatchCheckpoint,
    /// Players.
    pub players: Vec<Q2CtfPlayerCheckpoint>,
}

/// Capture CTF state (`captureQ2Ctf`).
pub fn capture_q2_ctf(game: &Q2GameServices) -> Q2CtfCheckpoint {
    let rules = &game.ctf.rules;
    let matched = &game.ctf.match_state;
    let mut ghosts: Vec<Q2CtfGhostCheckpoint> = matched
        .ghosts
        .values()
        .map(|ghost| Q2CtfGhostCheckpoint {
            code: ghost.code,
            team: ghost.team,
            name: ghost.name.clone(),
            actor: ghost.actor.as_ref().map(save_ctf_actor),
            score: ghost.score,
            deaths: ghost.deaths,
            kills: ghost.kills,
            captures: ghost.captures,
            base_defense: ghost.base_defense,
            carrier_defense: ghost.carrier_defense,
        })
        .collect();
    ghosts.sort_by(|left, right| left.code.cmp(&right.code));
    let mut players: Vec<Q2CtfPlayerCheckpoint> = game
        .ctf
        .states
        .iter()
        .map(|(actor, state)| {
            let grapple = if game.ctf.equipment.is_some() {
                game.equipment.ctf_states.get(actor).cloned().unwrap_or_default()
            } else {
                CtfGrappleState::default()
            };
            Q2CtfPlayerCheckpoint {
                actor: save_ctf_actor(actor),
                state: state.clone(),
                grapple: capture_ctf_grapple(&grapple),
            }
        })
        .collect();
    players.sort_by(|left, right| {
        (left.actor.slot, left.actor.generation).cmp(&(right.actor.slot, right.actor.generation))
    });
    Q2CtfCheckpoint {
        version: 1,
        rules: Q2CtfRulesCheckpoint {
            force_join: rules.force_join,
            competition: rules.competition,
            match_lock: rules.match_lock,
            election_percentage: rules.election_percentage,
            match_minutes: rules.match_minutes,
            setup_minutes: rules.setup_minutes,
            start_seconds: rules.start_seconds,
            capture_limit: rules.capture_limit,
            instant_weapons: rules.instant_weapons,
        },
        match_state: Q2CtfMatchCheckpoint {
            team1: matched.team1,
            team2: matched.team2,
            total1: matched.total1,
            total2: matched.total2,
            last_flag_capture: matched.last_flag_capture,
            last_capture_team: matched.last_capture_team,
            phase: matched.phase,
            match_time: matched.match_time,
            last_time: matched.last_time,
            election: matched.election.as_ref().map(|election| Q2CtfElectionCheckpoint {
                kind: election.kind,
                target: save_ctf_actor(&election.target),
                map: election.map.clone(),
                message: election.message.clone(),
                votes: election.votes,
                needed: election.needed,
                expires: election.expires,
            }),
            ghosts,
        },
        players,
    }
}

/// Restore CTF state (`restoreQ2Ctf`).
///
/// Shared actors, player records, inventory and the foundation restore first.
pub fn restore_q2_ctf(game: &mut Q2GameServices, checkpoint: &Q2CtfCheckpoint) {
    if let Some(equipment) = game.ctf.equipment {
        equipment.bind(game);
    }
    let mut players: Vec<(ActorId, Q2CtfPlayerState, CtfGrappleState)> = Vec::with_capacity(checkpoint.players.len());
    for entry in &checkpoint.players {
        let Some(owned) = game.host.actors().resolve_saved(entry.actor) else {
            panic!("CTF restore requires the existing shared player and source entity");
        };
        let actor = owned.id().clone();
        if game.entity(&actor).is_none() {
            panic!("CTF restore requires the existing shared player and source entity");
        }
        let hooks = super::ctf_hooks(game);
        if (hooks.player)(actor.clone(), game).is_none() {
            panic!("CTF restore requires the existing shared player and source entity");
        }
        let hook = restore_ctf_grapple(entry.grapple.clone(), game);
        players.push((actor, entry.state.clone(), hook));
    }
    game.ctf.states.clear();
    game.equipment.ctf_states.clear();
    for (actor, state, hook) in players {
        game.ctf.states.insert(actor.clone(), state);
        game.equipment.ctf_states.insert(actor, hook);
    }
    {
        let rules = &mut game.ctf.rules;
        rules.force_join = checkpoint.rules.force_join;
        rules.competition = checkpoint.rules.competition;
        rules.match_lock = checkpoint.rules.match_lock;
        rules.election_percentage = checkpoint.rules.election_percentage;
        rules.match_minutes = checkpoint.rules.match_minutes;
        rules.setup_minutes = checkpoint.rules.setup_minutes;
        rules.start_seconds = checkpoint.rules.start_seconds;
        rules.capture_limit = checkpoint.rules.capture_limit;
        rules.instant_weapons = checkpoint.rules.instant_weapons;
    }
    {
        let matched = &mut game.ctf.match_state;
        matched.team1 = checkpoint.match_state.team1;
        matched.team2 = checkpoint.match_state.team2;
        matched.total1 = checkpoint.match_state.total1;
        matched.total2 = checkpoint.match_state.total2;
        matched.last_flag_capture = checkpoint.match_state.last_flag_capture;
        matched.last_capture_team = checkpoint.match_state.last_capture_team;
        matched.phase = checkpoint.match_state.phase;
        matched.match_time = checkpoint.match_state.match_time;
        matched.last_time = checkpoint.match_state.last_time;
        matched.ghosts.clear();
    }
    for ghost in &checkpoint.match_state.ghosts {
        let actor = ghost.actor.map(|saved| game.host.actors().reference_saved(saved));
        game.ctf.match_state.ghosts.insert(
            ghost.code,
            super::types::Q2CtfGhost {
                code: ghost.code,
                team: ghost.team,
                name: ghost.name.clone(),
                actor,
                score: ghost.score,
                deaths: ghost.deaths,
                kills: ghost.kills,
                captures: ghost.captures,
                base_defense: ghost.base_defense,
                carrier_defense: ghost.carrier_defense,
            },
        );
    }
    game.ctf.match_state.election =
        checkpoint
            .match_state
            .election
            .as_ref()
            .map(|election| super::types::Q2CtfElection {
                kind: election.kind,
                target: game.host.actors().reference_saved(election.target),
                map: election.map.clone(),
                message: election.message.clone(),
                votes: election.votes,
                needed: election.needed,
                expires: election.expires,
            });
    if game.options.edition == Q2Edition::Classic {
        let predictions: Vec<(ActorId, bool)> = game
            .equipment
            .ctf_states
            .iter()
            .map(|(actor, state)| {
                (
                    actor.clone(),
                    state.grapple.is_some() && state.grapple_state == CtfGrapplePhase::Hang,
                )
            })
            .collect();
        let hooks = super::ctf_hooks(game);
        for (actor, suppressed) in predictions {
            (hooks.set_grapple_prediction)(actor, game, suppressed);
        }
    }
}
