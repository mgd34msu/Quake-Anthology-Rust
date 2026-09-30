//! Base content provider (`src/content/q1/base/provider.ts`).
//!
//! Base Quake campaign and boss behavior. Copyright (C) 1996-2022 id
//! Software LLC. GPL-2.0-or-later.
//!
//! The donor keys provider state off the game through `WeakMap`s. Rust
//! callbacks are bare function pointers, so per-game base state lives
//! in the module registry below, keyed by game address. Access runs
//! through [`update_base`], whose closure receives only the state:
//! game calls cannot run while the registry is locked, which rules out
//! reentrant deadlocks by construction. Flows that need both state and
//! the game clone the state out, operate, then store it back.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::{ActorId, OwnedActor, SavedActorId};
use qa_core::math::Vec3;

use crate::monsters::provider_text;
use crate::q1::base::creatures::Q1CreatureState;
use crate::q1::base::finales::q1_finale_text;
use crate::q1::base::map_entities::REMAINING_MAP_CLASSNAMES;
use crate::q1::base::monsters::BaseMonster;
use crate::q1::base::rules::{Q1IntermissionResult, Q1IntermissionRule, Q1LevelRules, Q1SpawnSelector};
use crate::q1::base::species::BASE_SPECIES;
use crate::q1::foundation::callbacks::{Q1CallbackHandlers, Q1StateExtension};
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity::Q1MoverState;
use crate::q1::foundation::entity_services::{Q1EntityServices, Q1Intermission};
use crate::q1::foundation::extensions::Q1PlayerExtension;
use crate::q1::foundation::gameplay::CombatTraits;
use crate::q1::foundation::host::Q1ReleaseHook;
use crate::q1::foundation::monsters::throw_gib;
use crate::q1::foundation::movers::door_down;
use crate::q1::foundation::types::{normalize, vadd, vscale, vsub, Q1BeamStyle, Q1Edition, Q1Event, ZERO};
use crate::q1::{q1_error, Q1Error};
use crate::value::{arr, boolean, int, num, obj, SaveJson, SaveReader};

/// Campaign flag and skill binding (`Q1CampaignBinding`).
pub trait Q1CampaignBinding: Send {
    /// Read episode flags.
    fn read_flags(&self) -> i32;
    /// Write episode flags.
    fn write_flags(&mut self, flags: i32);
    /// Set the skill level (`0..=3`).
    fn set_skill(&mut self, skill: i32);
}

/// Owned campaign binding (`Q1CampaignState`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1CampaignState {
    /// Episode flags.
    pub flags: i32,
    /// Skill level.
    pub skill: i32,
}

impl Q1CampaignState {
    /// Fresh binding.
    #[must_use]
    pub fn new(flags: i32, skill: i32) -> Self {
        Self { flags, skill }
    }
}

impl Default for Q1CampaignState {
    fn default() -> Self {
        Self { flags: 0, skill: 1 }
    }
}

impl Q1CampaignBinding for Q1CampaignState {
    fn read_flags(&self) -> i32 {
        self.flags
    }

    fn write_flags(&mut self, flags: i32) {
        self.flags = flags;
    }

    fn set_skill(&mut self, skill: i32) {
        self.skill = skill;
    }
}

/// Shared campaign binding. The base state, level rules and spawn
/// selector hold clones of one handle, mirroring the donor, which
/// passes a single binding to every owner.
#[derive(Clone)]
pub struct Q1CampaignHandle {
    inner: std::sync::Arc<Mutex<Box<dyn Q1CampaignBinding>>>,
}

impl Q1CampaignHandle {
    /// Share a binding.
    #[must_use]
    pub fn new(binding: Box<dyn Q1CampaignBinding>) -> Self {
        Self {
            inner: std::sync::Arc::new(Mutex::new(binding)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Box<dyn Q1CampaignBinding>> {
        self.inner.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Read episode flags.
    #[must_use]
    pub fn read_flags(&self) -> i32 {
        self.lock().read_flags()
    }

    /// Write episode flags.
    pub fn write_flags(&self, flags: i32) {
        self.lock().write_flags(flags);
    }

    /// Set the skill level.
    pub fn set_skill(&self, skill: i32) {
        self.lock().set_skill(skill);
    }
}

/// Base content options (`Q1BaseOptions`).
#[derive(Default)]
pub struct Q1BaseOptions {
    /// Campaign binding override.
    pub campaign: Option<Box<dyn Q1CampaignBinding>>,
    /// Whether the game is registered.
    pub registered: Option<bool>,
    /// Finale dismissal probe.
    pub finale_finished: Option<fn() -> bool>,
    /// Campaign completion.
    pub finish_campaign: Option<fn()>,
    /// Same-level changelevel probe.
    pub same_level: Option<fn() -> bool>,
    /// Level-exit observer.
    pub player_exited: Option<fn(ActorId)>,
    /// Official-campaign override.
    pub official_campaign: Option<bool>,
}

/// Kill-count rule (`Q1Base["killCountRules"]` value).
pub type Q1KillCountRule = for<'g> fn(&BaseMonster<'g>) -> bool;

/// Per-game base content state (`Q1Base`).
pub struct Q1BaseState {
    /// Creature controllers and cargo.
    pub creatures: Q1CreatureState,
    /// Campaign binding.
    pub campaign: Q1CampaignHandle,
    /// Whether the game is registered.
    pub registered: bool,
    /// Level and intermission rules.
    pub level_rules: Q1LevelRules,
    /// Spawn selector.
    pub spawn_selector: Q1SpawnSelector,
    /// Kill-count rules in registration order.
    pub kill_count_rules: Vec<(String, Q1KillCountRule)>,
    /// Chthon lightning end time in seconds.
    pub lightning_end: f64,
    /// Lightning electrodes, if armed.
    pub electrodes: Option<(ActorId, ActorId)>,
    /// Whether the Shub finale started.
    pub finale_started: bool,
    /// Whether the finale was dismissed.
    pub finale_dismissed: bool,
    /// Finale dismissal probe.
    pub finale_finished: Option<fn() -> bool>,
    /// Campaign completion.
    pub finish_campaign: Option<fn()>,
    /// Same-level changelevel probe.
    pub same_level: Option<fn() -> bool>,
    /// Level-exit observer.
    pub player_exited: Option<fn(ActorId)>,
}

fn registry() -> &'static Mutex<HashMap<usize, Q1BaseState>> {
    static STATES: OnceLock<Mutex<HashMap<usize, Q1BaseState>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn base_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

fn lock_registry() -> std::sync::MutexGuard<'static, HashMap<usize, Q1BaseState>> {
    registry().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Run a pure state operation. The closure receives only the state so
/// game calls (which may reenter base callbacks) cannot run under the
/// registry lock.
pub(crate) fn update_base<T>(game: &Q1EntityServices, op: impl FnOnce(&mut Q1BaseState) -> T) -> Result<T, Q1Error> {
    let mut states = lock_registry();
    let state = states
        .get_mut(&base_key(game))
        .ok_or_else(|| q1_error("Q1 base content was not registered"))?;
    Ok(op(state))
}

/// Whether base content is registered on a game.
#[must_use]
pub fn base_registered(game: &Q1EntityServices) -> bool {
    lock_registry().contains_key(&base_key(game))
}

/// Read a save-safe actor reference. Out-of-range slots saturate so
/// they never resolve, matching the donor, where they miss lookup.
pub(crate) fn saved_actor(reader: SaveReader) -> Result<SavedActorId, Q1Error> {
    let slot = reader.field("slot").integer(0)?;
    let generation = reader.field("generation").integer(0)?;
    Ok(SavedActorId {
        slot: u32::try_from(slot).unwrap_or(u32::MAX),
        generation: u32::try_from(generation).unwrap_or(u32::MAX),
    })
}

/// Resolve a saved owned actor or fail with the donor message.
pub(crate) fn resolve_saved_actor(game: &Q1EntityServices, reader: SaveReader) -> Result<OwnedActor, Q1Error> {
    let saved = saved_actor(reader.clone())?;
    game.host
        .actors
        .resolve_saved(&saved)
        .ok_or_else(|| Q1Error::from(reader.fail("missing saved actor")))
}

/// Base classnames in donor order (`Q1_BASE_CLASSNAMES`).
#[must_use]
pub fn q1_base_classnames() -> Vec<&'static str> {
    let mut classnames: Vec<&'static str> = BASE_SPECIES
        .iter()
        .flat_map(|species| species.classnames.iter().copied())
        .collect();
    classnames.extend(REMAINING_MAP_CLASSNAMES.iter().copied());
    classnames
}

fn level_stats_attach(game: &mut Q1EntityServices, player: &ActorId) -> Result<(), Q1Error> {
    let player = player.clone();
    update_base(game, |state| state.level_rules.reset_player(&player))
}

/// Register base content on a game (`registerQ1Base`). The game must
/// stay at a stable address while registered.
pub fn register_q1_base(game: &mut Q1EntityServices, options: Q1BaseOptions) -> Result<(), Q1Error> {
    if base_registered(game) {
        return Err(q1_error("Q1 creature services already registered"));
    }
    let campaign = Q1CampaignHandle::new(
        options
            .campaign
            .unwrap_or_else(|| Box::new(Q1CampaignState::new(0, game.options().skill))),
    );
    let registered = options.registered.unwrap_or(true);
    let official_campaign = options
        .official_campaign
        .unwrap_or_else(|| provider_text(&game.options().campaign).ends_with(":id1"));
    lock_registry().insert(
        base_key(game),
        Q1BaseState {
            creatures: Q1CreatureState::default(),
            campaign: campaign.clone(),
            registered,
            level_rules: Q1LevelRules::new(campaign.clone(), registered, official_campaign),
            spawn_selector: Q1SpawnSelector::new(campaign),
            kill_count_rules: Vec::new(),
            lightning_end: -1.0,
            electrodes: None,
            finale_started: false,
            finale_dismissed: false,
            finale_finished: options.finale_finished,
            finish_campaign: options.finish_campaign,
            same_level: options.same_level,
            player_exited: options.player_exited,
        },
    );
    game.register_player_extension(Q1PlayerExtension {
        id: String::from("q1:base-level-stats"),
        attach: Some(level_stats_attach),
        ..Default::default()
    })?;
    crate::q1::base::creatures::register_creature_callbacks(game)?;
    crate::q1::base::map_entities::register_map_callbacks(game)?;
    crate::q1::base::player::register_character_callbacks(game)?;
    game.named.register(
        "base:lightning_use",
        Q1CallbackHandlers {
            use_callback: Some(lightning_use),
            ..Default::default()
        },
    )?;
    game.named.register(
        "base:lightning_fire",
        Q1CallbackHandlers {
            action: Some(lightning_fire),
            ..Default::default()
        },
    )?;
    game.named.register(
        "base:finale_2",
        Q1CallbackHandlers {
            action: Some(finale_2),
            ..Default::default()
        },
    )?;
    game.named.register(
        "base:finale_3",
        Q1CallbackHandlers {
            action: Some(finale_3),
            ..Default::default()
        },
    )?;
    game.named.register(
        "base:finale_wait",
        Q1CallbackHandlers {
            action: Some(finale_wait),
            ..Default::default()
        },
    )?;
    game.named.register(
        "base:finale_6",
        Q1CallbackHandlers {
            action: Some(finale_6),
            ..Default::default()
        },
    )?;
    crate::q1::base::rules::register_level_callbacks(game)?;
    game.register_state_extension(Box::new(Q1BaseStateExtension))?;
    crate::q1::base::creatures::register_species(game)?;
    for classname in REMAINING_MAP_CLASSNAMES {
        game.register_spawn(classname, crate::q1::base::map_entities::spawn_remaining_map_actor)?;
    }
    game.register_release_hook(Rc::new(RefCell::new(Q1BaseReleaseHook)));
    Ok(())
}

/// Remove base content state from a game. Named callbacks and spawn
/// handlers stay registered; use a fresh game to re-register.
pub fn unregister_q1_base(game: &Q1EntityServices) {
    lock_registry().remove(&base_key(game));
}

/// RAII base registration for tests and scoped sessions.
pub struct Q1BaseGuard {
    key: usize,
}

impl Q1BaseGuard {
    /// Register base content, removing the state on drop.
    pub fn register(game: &mut Q1EntityServices, options: Q1BaseOptions) -> Result<Self, Q1Error> {
        register_q1_base(game, options)?;
        Ok(Self { key: base_key(game) })
    }
}

impl Drop for Q1BaseGuard {
    fn drop(&mut self) {
        lock_registry().remove(&self.key);
    }
}

/// Register a kill-count rule.
pub fn register_kill_count_rule(game: &Q1EntityServices, id: &str, rule: Q1KillCountRule) -> Result<(), Q1Error> {
    let id = id.to_string();
    update_base(game, |state| {
        if state.kill_count_rules.iter().any(|(candidate, _)| candidate == &id) {
            return Err(q1_error(format!("Duplicate Q1 kill-count rule {id}")));
        }
        state.kill_count_rules.push((id, rule));
        Ok(())
    })?
}

/// Whether a monster kill counts (`countMonsterKill`).
pub fn count_monster_kill(monster: &BaseMonster) -> Result<bool, Q1Error> {
    let rules = update_base(monster.game, |state| state.kill_count_rules.clone())?;
    Ok(rules.iter().all(|(_, rule)| rule(monster)))
}

/// Next Hell Knight melee frame (`nextHellKnightMelee`).
pub fn next_hell_knight_melee(game: &Q1EntityServices) -> Result<String, Q1Error> {
    update_base(game, |state| {
        crate::q1::base::creatures::next_hell_knight_melee(&mut state.creatures)
    })
}

/// Spawn an `event_lightning` entity.
pub fn spawn_lightning(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let use_callback = game.named.use_callback("base:lightning_use")?;
    game.update_entity(&id, |entity| entity.use_callback = Some(use_callback))
}

fn lightning_use(
    game: &mut Q1EntityServices,
    id: &ActorId,
    _other: Option<&ActorId>,
    activator: Option<&ActorId>,
) -> Result<(), Q1Error> {
    use_lightning(game, id, activator)
}

fn use_lightning(game: &mut Q1EntityServices, id: &ActorId, activator: Option<&ActorId>) -> Result<(), Q1Error> {
    let id = id.clone();
    let activator = activator.cloned();
    if update_base(game, |state| state.lightning_end)? >= game.time + 1.0 {
        return Ok(());
    }
    let mut electrodes: Vec<ActorId> = game
        .entity_ids()
        .into_iter()
        .filter(|candidate| {
            game.entity_ref(candidate)
                .is_some_and(|entity| entity.target == "lightning")
        })
        .collect();
    if electrodes.len() < 2 {
        return Err(q1_error("event_lightning is missing its two lightning electrodes"));
    }
    let second = electrodes.remove(1);
    let first = electrodes.remove(0);
    let states = (
        game.entity_ref(&first).map(|entity| entity.state),
        game.entity_ref(&second).map(|entity| entity.state),
    );
    if states.0 != states.1 || !matches!(states.0, Some(Q1MoverState::Top | Q1MoverState::Bottom)) {
        return Ok(());
    }
    game.cancel(&first);
    game.cancel(&second);
    let end = game.time + 1.0;
    update_base(game, |state| {
        state.electrodes = Some((first.clone(), second.clone()));
        state.lightning_end = end;
    })?;
    game.sound(
        &id,
        "misc/power.wav",
        crate::q1::foundation::types::Q1SoundChannel::Voice,
        1.0,
        1.0,
    )?;
    fire_lightning(game, &id)?;
    let boss = update_base(game, |state| {
        state
            .creatures
            .monsters
            .iter()
            .find(|(_, controller)| controller.species == crate::q1::foundation::entity::Q1MonsterSpecies::Boss)
            .map(|(actor, _)| actor.clone())
    })?;
    let Some(boss) = boss else { return Ok(()) };
    game.update_entity(&boss, |entity| {
        if let Some(monster) = entity.monster.as_mut() {
            monster.enemy = activator;
        }
    })?;
    if states.0 == Some(Q1MoverState::Top) && game.health(&boss) > 0.0 {
        game.sound(
            &boss,
            "boss1/pain.wav",
            crate::q1::foundation::types::Q1SoundChannel::Voice,
            1.0,
            1.0,
        )?;
        let health = game.health(&boss) - 1.0;
        let owned = game
            .entity_ref(&boss)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        game.host.combat.set_health(&owned, health)?;
        let mut monster = BaseMonster::load(game, &boss)?;
        let frame = if health >= 2.0 {
            "boss_shocka1"
        } else if health == 1.0 {
            "boss_shockb1"
        } else {
            "boss_shockc1"
        };
        monster.play(frame)?;
        monster.finish()?;
    }
    Ok(())
}

fn lightning_fire(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    fire_lightning(game, id)
}

fn fire_lightning(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let (electrodes, end) = update_base(game, |state| (state.electrodes.clone(), state.lightning_end))?;
    let Some((first, second)) = electrodes else {
        return Err(q1_error("Lightning has no electrodes"));
    };
    if game.time >= end {
        door_down(game, &first)?;
        return door_down(game, &second);
    }
    let a = game.body(&first)?;
    let b = game.body(&second)?;
    let center_a = vscale(vadd(a.bounds.min, a.bounds.max), 0.5);
    let center_b = vscale(vadd(b.bounds.min, b.bounds.max), 0.5);
    let p1 = Vec3 {
        x: center_a.x,
        y: center_a.y,
        z: a.origin.z + a.bounds.min.z - 16.0,
    };
    let raw = Vec3 {
        x: center_b.x,
        y: center_b.y,
        z: b.origin.z + b.bounds.min.z - 16.0,
    };
    let p2 = vsub(raw, vscale(normalize(vsub(raw, p1)), 100.0));
    game.host.emit(Q1Event::Beam {
        style: Q1BeamStyle::Lightning3,
        actor: game.world.clone().unwrap_or_else(|| id.clone()),
        start: p1,
        end: p2,
    });
    game.schedule(&id, 0.1, "base:lightning_fire")
}

fn finale_timer_shub(game: &Q1EntityServices, timer: &ActorId) -> Result<ActorId, Q1Error> {
    game.entity_ref(timer)
        .and_then(|entity| entity.owner.clone())
        .filter(|owner| game.entity_ref(owner).is_some())
        .ok_or_else(|| q1_error("Finale timer lost Shub"))
}

fn finale_2(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let shub = finale_timer_shub(game, &id)?;
    let origin = game.body(&shub)?;
    game.effect(
        crate::q1::foundation::types::Q1Effect::Teleport,
        vsub(
            origin.origin,
            Vec3 {
                x: 0.0,
                y: 100.0,
                z: 0.0,
            },
        ),
        None,
        1,
    );
    game.sound_simple(&shub, "misc/r_tele1.wav")?;
    game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 2,
    });
    game.schedule(&id, 2.0, "base:finale_3")
}

fn finale_3(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let shub = finale_timer_shub(game, &id)?;
    game.sound_simple(&shub, "boss2/death.wav")?;
    game.host.emit(Q1Event::Lightstyle {
        style: 0,
        pattern: String::from("abcdefghijklmlkjihgfedcb"),
    });
    game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 3,
    });
    let mut monster = BaseMonster::load(game, &shub)?;
    monster.controller.next_frame = String::from("old_thrash1");
    monster.delay(0.1)?;
    monster.finish()?;
    game.remove(&id)
}

fn finale_wait(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    if !has_finished_finale(game)? {
        return game.schedule(&id, 0.1, "base:finale_wait");
    }
    game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 5,
    });
    game.schedule(&id, 5.0, "base:finale_6")
}

fn finale_6(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 6,
    });
    if game.options().coop {
        game.travel("start", None);
    } else if let Some(finish) = update_base(game, |state| state.finish_campaign)? {
        finish();
    }
    game.remove(&id)
}

/// Start the Shub finale (`finale`).
pub fn base_finale(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    if update_base(monster.game, |state| state.finale_started)? {
        return Ok(());
    }
    update_base(monster.game, |state| {
        state.finale_started = true;
        state.finale_dismissed = false;
    })?;
    monster.count_kill()?;
    let id = monster.id.clone();
    monster.game.cancel(&id);
    let entities = monster.game.entity_ids();
    let position = entities
        .iter()
        .find(|candidate| {
            monster
                .game
                .entity_ref(candidate)
                .is_some_and(|entity| entity.classname == "info_intermission")
        })
        .cloned();
    let train = entities
        .iter()
        .find(|candidate| {
            monster
                .game
                .entity_ref(candidate)
                .is_some_and(|entity| entity.classname == "misc_teleporttrain")
        })
        .cloned();
    let (Some(position), Some(train)) = (position, train) else {
        return Err(q1_error("Q1 finale requires info_intermission and misc_teleporttrain"));
    };
    monster.game.remove(&train)?;
    let cause = monster.monster.enemy.clone();
    let exit_after = monster.game.time + 10_000_000.0;
    monster.game.intermission = Some(Q1Intermission {
        map: String::from("start"),
        cause,
        exit_after,
    });
    let camera_origin = monster.game.body(&position).map(|body| body.origin)?;
    let camera_angles = monster
        .game
        .entity_ref(&position)
        .map(|entity| entity.vector("mangle"))
        .unwrap_or(ZERO);
    let players = (monster.game.host.players)();
    for player in &players {
        let owned = monster.game.host.actors.resolve_owned(player);
        let body = monster.game.host.bodies.read(player);
        let (Some(owned), Some(body)) = (owned, body) else {
            continue;
        };
        monster.game.host.bodies.write(
            &owned,
            &crate::q1::foundation::gameplay::BodyState {
                origin: camera_origin,
                angles: camera_angles,
                velocity: ZERO,
                bounds: body.bounds,
                ground: body.ground,
            },
        )?;
        monster.game.host.bodies.link(&owned)?;
        let combat = monster
            .game
            .host
            .combat
            .read(owned.id())
            .ok_or_else(|| q1_error("Missing Q1 entity"))?;
        monster.game.host.combat.set_traits(
            &owned,
            CombatTraits {
                can_take_damage: false,
                mass: combat.mass,
                invulnerable: combat.invulnerable,
                team: combat.team,
                no_knockback: combat.no_knockback,
            },
        )?;
    }
    monster.game.host.emit(Q1Event::Intermission {
        origin: camera_origin,
        angles: camera_angles,
        map: String::from("start"),
        exit_after,
        track: 0,
    });
    monster.game.host.emit(Q1Event::Finale {
        text: String::new(),
        stage: 1,
    });
    if monster.game.options().edition == Q1Edition::Rerelease && monster.game.map_name == "end" {
        monster.game.host.emit(Q1Event::Achievement {
            player: None,
            id: String::from("ACH_DEFEAT_SHUB"),
        });
        if monster.game.options().skill == 3 {
            monster.game.host.emit(Q1Event::Achievement {
                player: None,
                id: String::from("ACH_DEFEAT_SHUB_NIGHTMARE"),
            });
        }
    }
    let timer = monster.game.create("finale_timer", None, None)?;
    monster.game.update_entity(&timer, |entity| entity.owner = Some(id))?;
    monster.game.schedule(&timer, 1.0, "base:finale_2")
}

/// Finish the Shub finale (`finishFinale`).
pub fn finish_finale(monster: &mut BaseMonster) -> Result<(), Q1Error> {
    let origin = monster.origin()?;
    monster.game.sound_simple(&monster.id.clone(), "boss2/pop2.wav")?;
    let mut z = 16.0f32;
    while z <= 144.0 {
        let mut x = -64.0f32;
        while x <= 64.0 {
            let mut y = -64.0f32;
            while y <= 64.0 {
                let roll = monster.game.host.random();
                let model = if roll < 0.3 {
                    "gib1"
                } else if roll < 0.6 {
                    "gib2"
                } else {
                    "gib3"
                };
                throw_gib(monster.game, vadd(origin, Vec3 { x, y, z }), model, -999.0)?;
                y += 32.0;
            }
            x += 32.0;
        }
        z += 96.0;
    }
    let edition = monster.game.options().edition;
    monster.game.host.emit(Q1Event::Finale {
        text: q1_finale_text(edition, "$qc_finale_end"),
        stage: 4,
    });
    let victory = monster.game.create("finale_player", None, None)?;
    monster.game.update_entity(&victory, |entity| {
        entity.model = String::from("progs/player.mdl");
        entity.frame = 1;
    })?;
    monster.game.set_body(
        &victory,
        &crate::q1::foundation::gameplay::BodyPatch {
            origin: Some(vsub(
                origin,
                Vec3 {
                    x: 32.0,
                    y: 264.0,
                    z: 0.0,
                },
            )),
            angles: Some(Vec3 {
                x: 0.0,
                y: 290.0,
                z: 0.0,
            }),
            ..Default::default()
        },
    )?;
    monster.game.link(&victory)?;
    let id = monster.id.clone();
    monster.game.remove(&id)?;
    monster.game.host.emit(Q1Event::Lightstyle {
        style: 0,
        pattern: String::from("m"),
    });
    if edition == Q1Edition::Classic {
        return Ok(());
    }
    let timer = monster.game.create("finale_wait", None, None)?;
    monster.game.schedule(&timer, 1.0, "base:finale_wait")
}

/// Dismiss the finale.
pub fn dismiss_finale(game: &Q1EntityServices) -> Result<(), Q1Error> {
    update_base(game, |state| state.finale_dismissed = true)
}

/// Reset finale dismissal.
pub fn reset_finale(game: &Q1EntityServices) -> Result<(), Q1Error> {
    update_base(game, |state| state.finale_dismissed = false)
}

/// Whether the finale finished.
pub fn has_finished_finale(game: &Q1EntityServices) -> Result<bool, Q1Error> {
    let (dismissed, probe) = update_base(game, |state| (state.finale_dismissed, state.finale_finished))?;
    if !dismissed {
        if let Some(finished) = probe {
            if finished() {
                update_base(game, |state| state.finale_dismissed = true)?;
                return Ok(true);
            }
        }
    }
    Ok(dismissed)
}

/// Read campaign flags.
pub fn campaign_read_flags(game: &Q1EntityServices) -> Result<i32, Q1Error> {
    update_base(game, |state| state.campaign.read_flags())
}

/// Write campaign flags.
pub fn campaign_write_flags(game: &Q1EntityServices, flags: i32) -> Result<(), Q1Error> {
    update_base(game, |state| state.campaign.write_flags(flags))
}

/// Set the campaign skill.
pub fn campaign_set_skill(game: &Q1EntityServices, skill: i32) -> Result<(), Q1Error> {
    update_base(game, |state| state.campaign.set_skill(skill))
}

/// Whether the game is registered.
pub fn base_registered_flag(game: &Q1EntityServices) -> Result<bool, Q1Error> {
    update_base(game, |state| state.registered)
}

/// Official-campaign flag, if any.
pub fn official_campaign_flag(game: &Q1EntityServices) -> Result<bool, Q1Error> {
    update_base(game, |state| state.level_rules.official_campaign())
}

/// Same-level probe, if any.
pub fn same_level_probe(game: &Q1EntityServices) -> Result<Option<fn() -> bool>, Q1Error> {
    update_base(game, |state| state.same_level)
}

/// Level-exit observer, if any.
pub fn player_exited_observer(game: &Q1EntityServices) -> Result<Option<fn(ActorId)>, Q1Error> {
    update_base(game, |state| state.player_exited)
}

/// Notify rules of a changelevel touch.
pub fn level_changelevel_touched(
    game: &mut Q1EntityServices,
    trigger: &ActorId,
    player: &ActorId,
) -> Result<(), Q1Error> {
    update_base(game, |state| state.level_rules.clone())?.changelevel_touched(game, trigger, player)
}

/// Travel through the level rules.
pub fn level_travel_to(game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<(), Q1Error> {
    update_base(game, |state| state.level_rules.clone())?.travel_to(game, map, cause)
}

/// Note a weapon attack.
pub fn level_note_attack(game: &Q1EntityServices, actor: &ActorId, axe_only: bool) -> Result<(), Q1Error> {
    let actor = actor.clone();
    update_base(game, |state| state.level_rules.note_attack(&actor, axe_only))
}

/// Note health damage.
pub fn level_note_damage(game: &Q1EntityServices, actor: &ActorId, health_damage: f64) -> Result<(), Q1Error> {
    let actor = actor.clone();
    update_base(game, |state| state.level_rules.note_damage(&actor, health_damage))
}

/// Reset one player's level statistics.
pub fn level_reset_player(game: &Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
    let actor = actor.clone();
    update_base(game, |state| state.level_rules.reset_player(&actor))
}

/// Register a source intermission rule.
pub fn level_register_rule(game: &Q1EntityServices, rule: Q1IntermissionRule) -> Result<(), Q1Error> {
    update_base(game, |state| state.level_rules.register_intermission_rule(rule))?
}

/// Begin an intermission to a map.
pub fn level_begin(game: &mut Q1EntityServices, map: &str, cause: Option<&ActorId>) -> Result<(), Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    rules.begin(game, map, cause)?;
    update_base(game, |state| state.level_rules = rules)
}

/// Begin a cutscene intermission.
pub fn level_begin_cutscene(
    game: &mut Q1EntityServices,
    map: &str,
    cause: Option<&ActorId>,
    exit_after: f64,
) -> Result<(), Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    rules.begin_cutscene(game, map, cause, exit_after);
    update_base(game, |state| state.level_rules = rules)
}

/// Check deathmatch limits.
pub fn level_check_limits(
    game: &mut Q1EntityServices,
    seconds: f64,
    scores: &[f64],
    timelimit_minutes: f64,
    fraglimit: f64,
) -> Result<bool, Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    let limited = rules.check_limits(game, seconds, scores, timelimit_minutes, fraglimit)?;
    update_base(game, |state| state.level_rules = rules)?;
    Ok(limited)
}

/// Request an intermission exit.
pub fn level_request_exit(
    game: &mut Q1EntityServices,
    seconds: f64,
    pressed: bool,
    same_level: bool,
) -> Result<Q1IntermissionResult, Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    let outcome = rules.request_exit(game, seconds, pressed, same_level)?;
    update_base(game, |state| state.level_rules = rules)?;
    Ok(outcome)
}

/// Exit on a mid-intermission client connection.
pub fn level_client_connected(
    game: &mut Q1EntityServices,
    seconds: f64,
    same_level: bool,
) -> Result<Q1IntermissionResult, Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    let outcome = rules.client_connected(game, seconds, same_level)?;
    update_base(game, |state| state.level_rules = rules)?;
    Ok(outcome)
}

/// Advance a finale.
pub fn level_advance_finale(game: &mut Q1EntityServices, seconds: f64) -> Result<Q1IntermissionResult, Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    let outcome = rules.advance_finale(game, seconds)?;
    update_base(game, |state| state.level_rules = rules)?;
    Ok(outcome)
}

/// Defer the intermission exit.
pub fn level_defer_exit(game: &mut Q1EntityServices, until_seconds: f64) -> Result<(), Q1Error> {
    let mut rules = update_base(game, |state| state.level_rules.clone())?;
    rules.defer_exit(game, until_seconds);
    update_base(game, |state| state.level_rules = rules)
}

/// Select a spawn point.
pub fn spawn_select(game: &mut Q1EntityServices, force_spawn: bool) -> Result<Option<ActorId>, Q1Error> {
    let mut selector = update_base(game, |state| state.spawn_selector.clone())?;
    let point = selector.select(game, force_spawn)?;
    update_base(game, |state| state.spawn_selector = selector)?;
    Ok(point)
}

/// Register a source spawn selection.
pub fn spawn_register_selection(
    game: &Q1EntityServices,
    id: &str,
    select: crate::q1::base::rules::Q1SpawnSelection,
) -> Result<(), Q1Error> {
    let id = id.to_string();
    update_base(game, |state| state.spawn_selector.register_selection(&id, select))?
}

/// Base state checkpoint extension (`q1:base`).
struct Q1BaseStateExtension;

impl Q1StateExtension for Q1BaseStateExtension {
    fn id(&self) -> &str {
        "q1:base"
    }

    fn capture(&self, game: &Q1EntityServices) -> Vec<u8> {
        let value = update_base(game, |state| {
            let mut members = vec![
                ("version", int(1)),
                ("flags", num(f64::from(state.campaign.read_flags()))),
                ("lightningEnd", num(state.lightning_end)),
                ("finaleStarted", boolean(state.finale_started)),
                ("finaleDismissed", boolean(state.finale_dismissed)),
                ("spawn", state.spawn_selector.capture()),
                ("rules", state.level_rules.capture()),
                (
                    "electrodes",
                    match &state.electrodes {
                        None => SaveJson::Null,
                        Some((first, second)) => arr(vec![saved_actor_json(first), saved_actor_json(second)]),
                    },
                ),
            ];
            members.extend(crate::q1::base::creatures::capture_creature_fields(&state.creatures));
            obj(members)
        })
        .expect("Q1 base content was not registered");
        encode_checkpoint_value(&value)
    }

    fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
        let value = decode_checkpoint_value(bytes)?;
        let root = SaveReader::at(&value, "q1:base");
        root.field("version").literal_i64(1)?;
        let flags = root.field("flags").number()?;
        let lightning_end = root.field("lightningEnd").number()?;
        let finale_started = root.field("finaleStarted").boolean()?;
        let finale_dismissed = root.field("finaleDismissed").boolean()?;
        let mut spawn = update_base(game, |state| state.spawn_selector.clone())?;
        spawn.restore(game, root.field("spawn"))?;
        let mut rules = update_base(game, |state| state.level_rules.clone())?;
        rules.restore(game, root.field("rules"))?;
        let electrodes = root.field("electrodes").nullable(|reader| {
            let entries = reader.list(|value| {
                let owned = resolve_saved_actor(game, value)?;
                Ok::<_, Q1Error>(game.entity_ref(owned.id()).map(|entity| entity.actor.id().clone()))
            })?;
            if entries.len() != 2 {
                return Err(Q1Error::from(reader.fail("missing lightning electrodes")));
            }
            let (Some(first), Some(second)) = (entries[0].clone(), entries[1].clone()) else {
                return Err(Q1Error::from(reader.fail("missing lightning electrodes")));
            };
            Ok::<_, Q1Error>((first, second))
        })?;
        let creatures = crate::q1::base::creatures::restore_creature_fields(game, root)?;
        update_base(game, |state| {
            state.campaign.write_flags(flags as i32);
            state.lightning_end = lightning_end;
            state.finale_started = finale_started;
            state.finale_dismissed = finale_dismissed;
            state.spawn_selector = spawn;
            state.level_rules = rules;
            state.electrodes = electrodes;
            state.creatures = creatures;
        })
    }

    fn clone_state(&mut self, _game: &mut Q1EntityServices, source: &ActorId, target: &ActorId) -> Result<(), Q1Error> {
        let (source, target) = (source.clone(), target.clone());
        update_base(_game, |state| {
            crate::q1::base::creatures::clone_creature_fields(&mut state.creatures, &source, &target)
        })
    }
}

fn saved_actor_json(actor: &ActorId) -> SaveJson {
    obj(vec![
        ("slot", int(i64::from(actor.slot()))),
        ("generation", int(i64::from(actor.generation()))),
    ])
}

/// Base actor-release cleanup.
struct Q1BaseReleaseHook;

impl Q1ReleaseHook for Q1BaseReleaseHook {
    fn on_release(&mut self, game: &mut Q1EntityServices, actor: &OwnedActor) {
        let id = actor.id().clone();
        let _ = update_base(game, |state| {
            crate::q1::base::creatures::release_creature_actor(&mut state.creatures, &id)
        });
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "id1"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn game() -> (
        Q1EntityServices,
        std::rc::Rc<std::cell::RefCell<crate::q1::foundation::host::mock::MockEvents>>,
    ) {
        let (host, events) = mock_host();
        (Q1EntityServices::new(host, options()).expect("game"), events)
    }

    #[test]
    fn registration_lifecycle_and_defaults() {
        let (mut game, _) = game();
        assert!(!base_registered(&game));
        register_q1_base(&mut game, Q1BaseOptions::default()).expect("register");
        assert!(base_registered(&game));
        let error = register_q1_base(&mut game, Q1BaseOptions::default()).expect_err("duplicate");
        assert_eq!(error.to_string(), "Q1 creature services already registered");
        assert_eq!(campaign_read_flags(&game).expect("flags"), 0);
        campaign_write_flags(&game, 7).expect("write");
        assert_eq!(campaign_read_flags(&game).expect("flags"), 7);
        assert!(base_registered_flag(&game).expect("registered"));
        let official = update_base(&game, |state| state.level_rules.clone()).expect("rules");
        assert_eq!(official.next_map, String::new());
        unregister_q1_base(&game);
        assert!(!base_registered(&game));
        assert!(update_base(&game, |_| ()).is_err());
    }

    #[test]
    fn guard_unregisters_and_classnames_cover_donor() {
        let (mut game, _) = game();
        {
            let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("guard");
            assert!(base_registered(&game));
            let classnames = q1_base_classnames();
            assert!(classnames.contains(&"monster_knight"));
            assert!(classnames.contains(&"monster_ogre_marksman"));
            assert!(classnames.contains(&"trigger_changelevel"));
            assert!(classnames.contains(&"misc_noisemaker"));
            assert_eq!(classnames.len(), 14 + REMAINING_MAP_CLASSNAMES.len());
            assert!(!has_finished_finale(&game).expect("finale"));
            dismiss_finale(&game).expect("dismiss");
            assert!(has_finished_finale(&game).expect("finale"));
            reset_finale(&game).expect("reset");
            assert!(!has_finished_finale(&game).expect("finale"));
        }
        assert!(!base_registered(&game));
    }

    #[test]
    fn kill_rules_and_melee_cycle() {
        let (mut game, _) = game();
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("guard");
        fn veto(_: &BaseMonster) -> bool {
            false
        }
        register_kill_count_rule(&game, "test", veto).expect("rule");
        assert!(register_kill_count_rule(&game, "test", veto).is_err());
        assert_eq!(next_hell_knight_melee(&game).expect("melee").as_str(), "hknight_slice1");
        assert_eq!(next_hell_knight_melee(&game).expect("melee").as_str(), "hknight_smash1");
        assert_eq!(next_hell_knight_melee(&game).expect("melee").as_str(), "hknight_watk1");
    }
}
