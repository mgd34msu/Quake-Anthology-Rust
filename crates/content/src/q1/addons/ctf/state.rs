//! Q1 CTF shared state (src/content/q1/addons/ctf/state.ts).

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

use qa_core::identity::{same_actor, ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::contract::{InventoryEntry, ItemId};
use crate::q1::addons::context::{addon_cvar, addon_player_number, set_addon_player_number, set_addon_vector};
use crate::q1::addons::ctf::types::{CtfRune, CtfStatus, CtfTeam, Q1CtfServices, CTF_RUNES};
use crate::q1::equipment::grapple::grapple_pulling;
use crate::q1::foundation::entity_services::Q1EntityServices;
use crate::q1::foundation::gameplay::{BodyPatch, BodyState};
use crate::q1::foundation::types::{Q1MessageArg, Q1SoundChannel};
use crate::q1::{q1_error, Q1Error};

/// Team color number (`teamNumber`): red 5, blue 14, none 0.
#[must_use]
pub fn team_number(team: Option<CtfTeam>) -> i64 {
    match team {
        Some(CtfTeam::Red) => 5,
        Some(CtfTeam::Blue) => 14,
        None => 0,
    }
}

/// Team for a color number (`numberTeam`).
#[must_use]
pub fn number_team(value: i64) -> Option<CtfTeam> {
    match value {
        5 => Some(CtfTeam::Red),
        14 => Some(CtfTeam::Blue),
        _ => None,
    }
}

/// Opposing team (`opposite`).
#[must_use]
pub fn opposite(team: CtfTeam) -> CtfTeam {
    match team {
        CtfTeam::Red => CtfTeam::Blue,
        CtfTeam::Blue => CtfTeam::Red,
    }
}

/// Rune inventory item (`runeItem`).
#[must_use]
pub fn rune_item(rune: CtfRune) -> ItemId {
    format!("q1:ctf/rune/{}", rune.as_str())
}

/// Deferred damage-stage side effect. Registered damage stages and the
/// grapple host run without game access, so stages queue their sounds,
/// word writes and reflection damage here; CTF entries drain the queue
/// with the game on the next call.
#[derive(Debug, Clone)]
pub(crate) enum CtfDeferred {
    /// Play a sound on an actor.
    Sound {
        /// Sound owner.
        actor: ActorId,
        /// Sound path.
        path: &'static str,
        /// Sound channel.
        channel: Q1SoundChannel,
    },
    /// Write an addon player word.
    PlayerNumber {
        /// Word owner.
        actor: ActorId,
        /// Word name.
        name: String,
        /// Word value.
        value: f64,
    },
    /// Reflect teammate damage back to the attacker.
    Reflect {
        /// Reflecting attacker.
        attacker: ActorId,
        /// Original inflictor, if any.
        inflictor: Option<ActorId>,
        /// Reflected damage.
        damage: f64,
    },
}

/// Synced snapshot for game-less damage stages and the grapple host.
/// CTF entries refresh it with [`sync_ctf_policy`]; stages only read.
#[derive(Debug, Default)]
pub(crate) struct CtfPolicy {
    /// Teamplay bits.
    pub teamplay: i64,
    /// Raw teamplay cvar for exact float comparisons.
    pub teamplay_raw: f64,
    /// Whether the map is `start`.
    pub start_map: bool,
    /// Last sync time in seconds.
    pub time: f64,
    /// Admitted players.
    pub players: HashSet<ActorId>,
    /// Carried rune per player.
    pub runes: HashMap<ActorId, Option<CtfRune>>,
    /// Last team per player.
    pub lastteam: HashMap<ActorId, Option<CtfTeam>>,
    /// Live combat team per player.
    pub teams: HashMap<ActorId, Option<CtfTeam>>,
    /// Owners currently pulling a hook.
    pub pulling: HashSet<ActorId>,
    /// Actors carrying a flag.
    pub carried: HashSet<ActorId>,
    /// Resistance sound throttle per target.
    pub resistance_throttle: HashMap<ActorId, f64>,
}

/// Per-game CTF registration.
pub(crate) struct CtfRegistration {
    /// Session services.
    pub services: Box<dyn Q1CtfServices>,
    /// Whether the native Threewave grapple drives hooks.
    pub native_grapple: bool,
    /// Whether the shared grapple selection is disabled.
    pub shared_disabled: bool,
    /// Synced stage policy.
    pub policy: CtfPolicy,
    /// Queued stage side effects.
    pub pending: Vec<CtfDeferred>,
}

fn registry() -> &'static Mutex<HashMap<usize, CtfRegistration>> {
    static STATES: OnceLock<Mutex<HashMap<usize, CtfRegistration>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<usize, CtfRegistration>> {
    registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Registry key for a game.
pub(crate) fn ctf_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

/// Register per-game CTF state, overwriting any previous entry.
pub(crate) fn register_ctf_state(
    game: &mut Q1EntityServices,
    services: Box<dyn Q1CtfServices>,
    native_grapple: bool,
    shared_disabled: bool,
) {
    lock_registry().insert(
        ctf_key(game),
        CtfRegistration {
            services,
            native_grapple,
            shared_disabled,
            policy: CtfPolicy::default(),
            pending: Vec::new(),
        },
    );
}

/// Run a pure registration operation.
pub(crate) fn update_ctf<T>(game: &Q1EntityServices, op: impl FnOnce(&mut CtfRegistration) -> T) -> Result<T, Q1Error> {
    let mut states = lock_registry();
    let state = states
        .get_mut(&ctf_key(game))
        .ok_or_else(|| q1_error("Q1 CTF was not registered"))?;
    Ok(op(state))
}

/// Run a registration operation from a game-less callback. Returns
/// `None` when the game has no registration.
pub(crate) fn ctf_by_key<T>(key: usize, op: impl FnOnce(&mut CtfRegistration) -> T) -> Option<T> {
    lock_registry().get_mut(&key).map(op)
}

/// Run a session services operation. The closure receives only the
/// services: game calls cannot run while the registry is locked, which
/// rules out reentrant deadlocks by construction.
pub(crate) fn with_ctf_services<T>(
    game: &Q1EntityServices,
    op: impl FnOnce(&mut dyn Q1CtfServices) -> T,
) -> Result<T, Q1Error> {
    update_ctf(game, |state| op(state.services.as_mut()))
}

/// Whether the native grapple drives hooks (`nativeGrappleEnabled`).
pub(crate) fn native_grapple_enabled(game: &Q1EntityServices) -> Result<bool, Q1Error> {
    update_ctf(game, |state| state.native_grapple)
}

/// Whether the shared grapple selection is disabled.
pub(crate) fn shared_grapple_disabled(game: &Q1EntityServices) -> Result<bool, Q1Error> {
    update_ctf(game, |state| state.shared_disabled)
}

/// Teamplay cvar (`teamplay`).
pub(crate) fn ctf_teamplay(game: &Q1EntityServices) -> Result<f64, Q1Error> {
    addon_cvar(game, "teamplay")
}

/// Teamplay cvar truncated to flag bits.
pub(crate) fn ctf_teamplay_bits(game: &Q1EntityServices) -> Result<i64, Q1Error> {
    Ok(ctf_teamplay(game)? as i64)
}

/// Whether the map is `start` (`startMap`).
#[must_use]
pub(crate) fn ctf_start_map(game: &Q1EntityServices) -> bool {
    game.map_name == "start"
}

/// World actor or fail with the donor message (`world`).
pub(crate) fn ctf_world(game: &Q1EntityServices) -> Result<ActorId, Q1Error> {
    game.world.clone().ok_or_else(|| q1_error("CTF map has no world actor"))
}

/// Owned actor or fail with the donor message (`owner`).
pub(crate) fn ctf_owner(game: &Q1EntityServices, actor: &ActorId) -> Result<OwnedActor, Q1Error> {
    game.host
        .actors
        .resolve_owned(actor)
        .ok_or_else(|| q1_error("CTF player is no longer admitted"))
}

/// Shared body or fail with the donor message (`body`).
pub(crate) fn ctf_body(game: &Q1EntityServices, actor: &ActorId) -> Result<BodyState, Q1Error> {
    game.host
        .bodies
        .read(actor)
        .ok_or_else(|| q1_error("CTF actor has no shared body"))
}

/// Patch a shared body and link it (`writeBody`).
pub(crate) fn ctf_write_body(game: &mut Q1EntityServices, actor: &ActorId, patch: &BodyPatch) -> Result<(), Q1Error> {
    let owner = ctf_owner(game, actor)?;
    let patched = patch.apply_to(&ctf_body(game, actor)?);
    game.host.bodies.write(&owner, &patched)?;
    game.host.bodies.link(&owner)?;
    Ok(())
}

/// Read a `ctf.*` player word (`number`).
pub(crate) fn ctf_number(game: &Q1EntityServices, actor: &ActorId, name: &str) -> Result<f64, Q1Error> {
    addon_player_number(game, actor, &format!("ctf.{name}"))
}

/// Write a `ctf.*` player word (`set`).
pub(crate) fn ctf_set(game: &mut Q1EntityServices, actor: &ActorId, name: &str, value: f64) -> Result<(), Q1Error> {
    set_addon_player_number(game, actor, &format!("ctf.{name}"), value)
}

/// Live combat team (`team`).
pub(crate) fn ctf_team(game: &Q1EntityServices, actor: &ActorId) -> Option<CtfTeam> {
    game.host
        .combat
        .read(actor)
        .and_then(|combat| combat.team)
        .as_deref()
        .and_then(CtfTeam::parse)
}

/// Last admitted team (`lastTeam`).
pub(crate) fn ctf_last_team(game: &Q1EntityServices, actor: &ActorId) -> Result<Option<CtfTeam>, Q1Error> {
    Ok(number_team(ctf_number(game, actor, "lastteam")? as i64))
}

/// Team flag entity, if any (`flag`).
pub(crate) fn ctf_flag(game: &Q1EntityServices, team: CtfTeam) -> Option<ActorId> {
    let classname = match team {
        CtfTeam::Red => "item_flag_team1",
        CtfTeam::Blue => "item_flag_team2",
    };
    game.entity_ids()
        .into_iter()
        .find(|id| game.entity_ref(id).is_some_and(|entity| entity.classname == classname))
}

/// Team of a flag entity (`flagTeam`).
pub(crate) fn ctf_flag_team(game: &Q1EntityServices, flag: &ActorId) -> CtfTeam {
    if game
        .entity_ref(flag)
        .is_some_and(|entity| entity.classname == "item_flag_team1")
    {
        CtfTeam::Red
    } else {
        CtfTeam::Blue
    }
}

/// Flag carried by an actor, if any (`carried`).
pub(crate) fn ctf_carried(game: &Q1EntityServices, actor: &ActorId) -> Option<ActorId> {
    game.entity_ids().into_iter().find(|id| {
        game.entity_ref(id).is_some_and(|entity| {
            entity.classname.starts_with("item_flag_team")
                && entity.count == 1.0
                && entity.owner.as_ref().is_some_and(|owner| same_actor(owner, actor))
        })
    })
}

/// Live hook owned by an actor, if any (`hook`).
pub fn ctf_hook(game: &Q1EntityServices, actor: &ActorId) -> Option<ActorId> {
    if update_ctf(game, |state| state.native_grapple).unwrap_or(false) {
        crate::q1::equipment::grapple::grapple_hook(game, actor)
    } else {
        None
    }
}

/// Rune carried by an actor, if any (`rune`).
pub(crate) fn ctf_rune(game: &Q1EntityServices, actor: &ActorId) -> Option<CtfRune> {
    CTF_RUNES
        .into_iter()
        .find(|rune| game.host.inventory.count(actor, &rune_item(*rune)) > 0.0)
}

/// Whether an owner is pulling a hook (`grapplePulling`).
#[must_use]
pub(crate) fn ctf_grapple_pulling(game: &Q1EntityServices, actor: &ActorId) -> bool {
    if update_ctf(game, |state| state.native_grapple).unwrap_or(false) {
        grapple_pulling(game, actor)
    } else {
        false
    }
}

/// Configure an inventory entry (`grant`).
pub(crate) fn ctf_grant(
    game: &mut Q1EntityServices,
    actor: &ActorId,
    item: &str,
    count: f64,
    capacity: f64,
) -> Result<(), Q1Error> {
    let owner = ctf_owner(game, actor)?;
    game.host.inventory.configure(
        &owner,
        &InventoryEntry {
            item: item.to_string(),
            count,
            capacity,
            count_policy: None,
        },
    )
}

/// Broadcast a keyed message to every player (`announce`).
pub(crate) fn ctf_announce(
    game: &mut Q1EntityServices,
    key: &str,
    actor: Option<&ActorId>,
    extra: &str,
) -> Result<(), Q1Error> {
    let mut args = Vec::new();
    if let Some(actor) = actor {
        let name = with_ctf_services(game, |services| services.name(actor))?;
        args.push(Q1MessageArg::Text(name));
    }
    if !extra.is_empty() {
        args.push(Q1MessageArg::Text(extra.to_string()));
    }
    for player in (game.host.players)() {
        game.message(Some(&player), key, false, args.clone());
    }
    Ok(())
}

/// Push scoreboard status to one player or every player (`update`).
pub(crate) fn ctf_update(game: &mut Q1EntityServices, player: Option<&ActorId>) -> Result<(), Q1Error> {
    let mut flags = 0;
    for (index, team) in [CtfTeam::Red, CtfTeam::Blue].into_iter().enumerate() {
        let flag = ctf_flag(game, team);
        let bits = match flag {
            None => 1,
            Some(flag) => {
                let count = game.entity_ref(&flag).map(|entity| entity.count as i32).unwrap_or(0);
                1 << count
            }
        };
        flags |= bits << (index * 3);
    }
    let targets: Vec<ActorId> = match player {
        Some(player) => vec![player.clone()],
        None => (game.host.players)(),
    };
    for actor in &targets {
        let rune = ctf_rune(game, actor);
        let ordinal = rune.map_or(-1, |rune| rune.ordinal() as i32);
        let status = CtfStatus {
            red: with_ctf_services(game, |services| services.captures(CtfTeam::Red))?,
            blue: with_ctf_services(game, |services| services.captures(CtfTeam::Blue))?,
            flags,
            rune_items: if ordinal < 0 { 0 } else { 32 << ordinal },
        };
        with_ctf_services(game, |services| services.status(actor, status))?;
    }
    Ok(())
}

/// Refresh the game-less stage policy from live state. CTF entries
/// call this before game-less callbacks can observe them.
pub(crate) fn sync_ctf_policy(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let teamplay_raw = ctf_teamplay(game)?;
    let mut players: HashSet<ActorId> = game.players.keys().cloned().collect();
    players.extend((game.host.players)());
    let mut runes = HashMap::new();
    let mut lastteam = HashMap::new();
    let mut teams = HashMap::new();
    let mut pulling = HashSet::new();
    for player in &players {
        runes.insert(player.clone(), ctf_rune(game, player));
        lastteam.insert(player.clone(), ctf_last_team(game, player)?);
        teams.insert(player.clone(), ctf_team(game, player));
        if ctf_grapple_pulling(game, player) {
            pulling.insert(player.clone());
        }
    }
    let mut carried = HashSet::new();
    for id in game.entity_ids() {
        let carrier = game.entity_ref(&id).and_then(|entity| {
            if entity.classname.starts_with("item_flag_team") && entity.count == 1.0 {
                entity.owner.clone()
            } else {
                None
            }
        });
        if let Some(carrier) = carrier {
            carried.insert(carrier);
        }
    }
    let policy = CtfPolicy {
        teamplay: teamplay_raw as i64,
        teamplay_raw,
        start_map: ctf_start_map(game),
        time: game.time,
        players,
        runes,
        lastteam,
        teams,
        pulling,
        carried,
        resistance_throttle: update_ctf(game, |state| state.policy.resistance_throttle.clone())?,
    };
    update_ctf(game, |state| {
        state.policy = policy;
    })
}

/// Apply queued damage-stage side effects in order.
pub(crate) fn drain_ctf_deferred(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    let pending = update_ctf(game, |state| std::mem::take(&mut state.pending))?;
    for action in &pending {
        match action {
            CtfDeferred::Sound { actor, path, channel } => {
                if game.host.bodies.read(actor).is_none() {
                    continue;
                }
                game.sound(actor, path, *channel, 1.0, 1.0)?;
            }
            CtfDeferred::PlayerNumber { actor, name, value } => {
                if game.host.actors.resolve_owned(actor).is_none() {
                    continue;
                }
                set_addon_player_number(game, actor, name, *value)?;
            }
            CtfDeferred::Reflect {
                attacker,
                inflictor,
                damage,
            } => {
                game.damage(
                    attacker,
                    inflictor.as_ref(),
                    Some(attacker),
                    *damage,
                    &crate::q1::foundation::entity_services::Q1DamageParams {
                        death_type: String::from("ctf:reflection"),
                        ..Default::default()
                    },
                );
            }
        }
    }
    Ok(())
}

/// Write a `ctf.*` entity vector (covers `setVector` uses).
pub(crate) fn ctf_set_entity_vector(
    game: &mut Q1EntityServices,
    id: &ActorId,
    name: &str,
    value: Vec3,
) -> Result<(), Q1Error> {
    set_addon_vector(game, id, name, value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::{attach_test_player, register_test_addons, Q1AddonProgram};
    use crate::q1::addons::ctf::types::{neutral_ctf_input, FakeCtfServices};
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::missionpacks::types::test_game;

    fn setup(game: &mut Q1EntityServices) -> (Q1BaseGuard, ActorId) {
        let guard = Q1BaseGuard::register(game, Q1BaseOptions::default()).expect("base");
        register_test_addons(game, Q1AddonProgram::Ctf);
        let (services, _) = FakeCtfServices::new();
        register_ctf_state(game, Box::new(services), true, false);
        let player = attach_test_player(game);
        (guard, player)
    }

    #[test]
    fn team_numbers_and_rune_items_match_donor() {
        assert_eq!(team_number(Some(CtfTeam::Red)), 5);
        assert_eq!(team_number(Some(CtfTeam::Blue)), 14);
        assert_eq!(team_number(None), 0);
        assert_eq!(number_team(5), Some(CtfTeam::Red));
        assert_eq!(number_team(14), Some(CtfTeam::Blue));
        assert_eq!(number_team(1), None);
        assert_eq!(opposite(CtfTeam::Red), CtfTeam::Blue);
        assert_eq!(rune_item(CtfRune::Haste), "q1:ctf/rune/haste");
    }

    #[test]
    fn words_grants_and_updates_round_trip() {
        let mut game = test_game();
        let (_guard, player) = setup(&mut game);
        assert_eq!(ctf_number(&game, &player, "lastteam"), Ok(0.0));
        ctf_set(&mut game, &player, "lastteam", 5.0).expect("set");
        assert_eq!(ctf_last_team(&game, &player), Ok(Some(CtfTeam::Red)));
        ctf_grant(&mut game, &player, &rune_item(CtfRune::Strength), 1.0, 1.0).expect("grant");
        assert_eq!(ctf_rune(&game, &player), Some(CtfRune::Strength));
        ctf_update(&mut game, Some(&player)).expect("update");
        sync_ctf_policy(&mut game).expect("sync");
        drain_ctf_deferred(&mut game).expect("drain");
        let _ = neutral_ctf_input();
    }
}
