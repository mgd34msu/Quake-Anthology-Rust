//! Q1 source composition (`src/content/composition/q1/runtime.ts`).
//!
//! Registers one official source program on the session's existing
//! world and actors. The Rust port keeps composition state beside the
//! game instead of inside a borrowed foundation: session hooks that
//! cannot reach the game queue pending actions, drained after every
//! composition entry point. Hooks firing from outer-driven game
//! frames (rogue team enforcement, horde waves) therefore apply on
//! the next composition call.
//!
//! Porting notes against the donor constructor and frame:
//! - `Q1BaseOptions` hooks are context-free fn pointers, so the
//!   `same_level`, `player_exited`, and `finish_campaign` probes stay
//!   unwired: changelevel always travels to the named map, level
//!   exits emit no notice, and the Shub finale cannot finish the
//!   campaign. The finale acknowledgement is polled from the frame
//!   phase instead, which is equivalent because dismissal is only
//!   read while a finale is active and every finale resets it first.
//! - Attack/jump input is cached per client: the composition is the
//!   only production writer of the held flags, so the cache matches
//!   the live player state exactly.
//! - `skill`, `teamplay`, and `gravity` snapshot their cvars at
//!   creation; the donor reads them through live option getters.
//! - CTF character poses map onto the source pose; grapple selection
//!   snapshots across CTF registration, which only reads the
//!   selection and native-slot probe.

use std::cell::RefCell;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex, MutexGuard};

use qa_core::identity::{same_actor, ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::bsp::Q1Map;
use crate::contract::{GrappleMechanic, GrappleSelection, ItemId, SharedGrappleControl};
use crate::q1::addons::commands::{frame_q1_addon_player, handle_q1_addon_impulse};
use crate::q1::addons::context::{
    addon_key, addon_player_number_by_key, frame_addons, register_addon_context, Q1AddonCheatCategory, Q1AddonContext,
    Q1AddonProgram, Q1AddonServices,
};
use crate::q1::addons::ctf::travel::{capture_q1_ctf_travel, decode_q1_ctf_travel, new_q1_ctf_travel};
use crate::q1::addons::ctf::types::{CtfInput, CtfPromptChoice, CtfTeam, Q1CtfServices};
use crate::q1::addons::ctf::{
    after_physics as ctf_after_physics, character_pose as ctf_character_pose, death as ctf_death,
    disconnect_player as ctf_disconnect_player, fall_damage_allowed as ctf_fall_damage_allowed, impulse as ctf_impulse,
    player_frame as ctf_player_frame, register_ctf, select_spawn as ctf_select_spawn, spawn_player as ctf_spawn_player,
    suicide as ctf_suicide,
};
use crate::q1::addons::horde::types::Q1HordeServices;
use crate::q1::addons::horde::{horde_request_respawn, horde_restore_keys, horde_teammate_killed, register_q1_horde};
use crate::q1::addons::items::{handle_mg3_item_impulse, mg3_hammer_body_frame};
use crate::q1::addons::register_q1_campaign_addons;
use crate::q1::addons::travel::{
    admit_q1_addon_travel, capture_q1_addon_travel, decode_q1_addon_travel, new_q1_addon_travel,
};
use crate::q1::base::player::{Q1CharacterPresentation, Q1CharacterSourcePose};
use crate::q1::base::projectiles::{drop_backpack, BackpackDrop};
use crate::q1::base::provider::{
    campaign_read_flags, dismiss_finale, level_check_limits, level_client_connected, level_note_attack,
    level_note_damage, level_request_exit, level_reset_player, register_q1_base, reset_finale, spawn_select,
    Q1BaseOptions, Q1CampaignBinding,
};
use crate::q1::base::rules::{
    q1_client_notice, q1_obituary, Q1ClientEvent, Q1DeathWater, Q1IntermissionResult, Q1ObituaryActor, Q1ObituaryInput,
    Q1SourceFinale,
};
use crate::q1::base::travel::{admit_q1_travel, capture_q1_travel, decode_q1_travel, new_q1_travel, Q1TravelState};
use crate::q1::composition::clients::Q1SourceClient;
use crate::q1::composition::clients::Q1SourceClients;
use crate::q1::composition::commands::{base_q1_impulse, q1_weapon_impulse};
use crate::q1::composition::types::{
    Q1ClientAdmission, Q1CompositionCheatCategory, Q1CompositionEvent, Q1CompositionPromptChoice,
    Q1CompositionServices, Q1SourceInput, Q1SourceProgram, Q1SourceSelection,
};
use crate::q1::foundation::callbacks::Q1StateExtension;
use crate::q1::foundation::entity_services::{Q1EntityServices, Q1PlayerInput};
use crate::q1::foundation::gameplay::{
    AttackCause, DamageDecision, DamagePreparation, DamageReaction, EnvironmentHazard, Q1DamageSourceEffects,
    Q1LethalHealth, Q1LethalReaction,
};
use crate::q1::foundation::host::{Q1FoundationHost, Q1ReleaseHook};
use crate::q1::foundation::runtime::Q1SpawnReport;
use crate::q1::foundation::types::{
    Q1Edition, Q1Event, Q1FoundationOptions, Q1MessageArg, Q1MoveType, Q1Powerup, Q1PrecacheProgram, Q1Solid, Q1Weapon,
    WEAPONS,
};
use crate::q1::missionpacks::commands::CheatArsenalCategory;
use crate::q1::missionpacks::runtime::{register_q1_mission_pack, Q1MissionPackOptions, Q1MissionPackRuntime};
use crate::q1::missionpacks::types::Q1MissionPack;
use crate::q1::{q1_error, Q1Error};

/// Finale button acknowledgement (`QcFinaleAcknowledgement`, donor
/// `src/compat/qc/presentation-host.ts`).
#[derive(Debug, Default)]
struct FinaleAcknowledgement {
    /// Held buttons by actor.
    held: HashMap<ActorId, bool>,
    /// Last poll time in seconds.
    last_poll: Option<f64>,
    /// Whether acknowledged.
    acknowledged: bool,
}

impl FinaleAcknowledgement {
    /// Fresh acknowledgement.
    fn new() -> Self {
        Self::default()
    }

    /// Reset the acknowledgement (`reset`).
    fn reset(&mut self) {
        self.last_poll = None;
        self.acknowledged = false;
        self.held.clear();
    }

    /// Poll button edges (`poll`).
    fn poll(&mut self, seconds: f64, buttons: &HashMap<ActorId, bool>) -> bool {
        if self.last_poll.is_none_or(|last| seconds < last || seconds - last > 1.0) {
            self.acknowledged = false;
            self.held.clear();
            for (actor, down) in buttons {
                self.held.insert(actor.clone(), *down);
            }
        }
        self.last_poll = Some(seconds);
        self.held.retain(|actor, _| buttons.contains_key(actor));
        for (actor, down) in buttons {
            if *down && self.held.get(actor) != Some(&true) {
                self.acknowledged = true;
            }
            self.held.insert(actor.clone(), *down);
        }
        self.acknowledged
    }

    /// Dismiss the finale (`dismiss`).
    fn dismiss(&mut self, seconds: f64) {
        self.last_poll = Some(seconds);
        self.acknowledged = true;
    }
}

/// Deferred game-reaching hook action.
#[derive(Debug, Clone, PartialEq, Eq)]
enum PendingComposition {
    /// Respawn at a spot.
    Respawn {
        /// Respawned actor.
        actor: ActorId,
        /// Forced spot, if any.
        spot: Option<ActorId>,
    },
    /// Disconnect the actor.
    Disconnect {
        /// Disconnected actor.
        actor: ActorId,
    },
    /// Apply player settings (combat traits and autoswitch).
    PlayerSettings {
        /// Updated actor.
        actor: ActorId,
    },
}

/// Shared composition state behind the session hooks.
struct Inner {
    /// Admitted clients.
    clients: Q1SourceClients,
    /// Selected session services.
    services: Box<dyn Q1CompositionServices>,
    /// Deferred hook actions.
    pending: VecDeque<PendingComposition>,
    /// Campaign binding shared with the base content.
    campaign: Box<dyn Q1CampaignBinding>,
    /// Finale acknowledgement.
    finale: FinaleAcknowledgement,
}

/// Lock the shared state, recovering from poisoning like the sibling
/// registries.
fn lock_inner(shared: &Arc<Mutex<Inner>>) -> MutexGuard<'_, Inner> {
    shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Addon services bridge (`addonServices`).
struct AddonBridge {
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
}

impl Q1AddonServices for AddonBridge {
    fn cheat_arsenal(&mut self, actor: &ActorId, category: Q1AddonCheatCategory) -> bool {
        let category = match category {
            Q1AddonCheatCategory::Weapons => Some(Q1CompositionCheatCategory::Weapons),
            Q1AddonCheatCategory::Ammo => Some(Q1CompositionCheatCategory::Ammo),
        };
        lock_inner(&self.shared).services.cheat_arsenal(actor, category)
    }

    fn emit(&mut self, event: crate::q1::addons::context::Q1AddonEvent) {
        lock_inner(&self.shared).services.emit(Q1CompositionEvent::Addon(event));
    }

    fn is_monster(&mut self, game: &Q1EntityServices, actor: &ActorId) -> bool {
        game.entity_ref(actor)
            .is_some_and(|entity| entity.movement_flags & 32 != 0)
    }

    fn cvar(&mut self, name: &str) -> f64 {
        lock_inner(&self.shared).services.cvar(name)
    }

    fn set_cvar(&mut self, name: &str, value: &str) {
        lock_inner(&self.shared).services.set_cvar(name, value);
    }
}

/// CTF services bridge (`ctfServices`). Client lookups require
/// admission with the donor message; game-reaching respawns and
/// disconnects defer to the drain.
struct CtfBridge {
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
}

impl CtfBridge {
    /// Lock the shared state.
    fn lock(&self) -> MutexGuard<'_, Inner> {
        lock_inner(&self.shared)
    }
}

impl Q1CtfServices for CtfBridge {
    fn name(&mut self, actor: &ActorId) -> String {
        self.lock()
            .clients
            .require(actor)
            .expect("Q1 source client is not admitted")
            .name()
    }

    fn is_bot(&mut self, actor: &ActorId) -> bool {
        self.lock().services.selected_player(actor).is_bot
    }

    fn score(&mut self, actor: &ActorId) -> f64 {
        self.lock()
            .clients
            .require(actor)
            .expect("Q1 source client is not admitted")
            .frags
    }

    fn add_score(&mut self, actor: &ActorId, delta: f64) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        inner
            .clients
            .add_score(&mut *inner.services, actor, delta)
            .expect("Q1 source client is not admitted");
    }

    fn captures(&mut self, team: CtfTeam) -> i32 {
        let inner = self.lock();
        match team {
            CtfTeam::Red => inner.clients.red_captures,
            CtfTeam::Blue => inner.clients.blue_captures,
        }
    }

    fn add_capture(&mut self, team: CtfTeam) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let total = match team {
            CtfTeam::Red => {
                inner.clients.red_captures += 1;
                inner.clients.red_captures
            }
            CtfTeam::Blue => {
                inner.clients.blue_captures += 1;
                inner.clients.blue_captures
            }
        };
        inner.services.emit(Q1CompositionEvent::CtfCapture { team, total });
    }

    fn input(&mut self, actor: &ActorId) -> CtfInput {
        let mut guard = self.lock();
        let inner = &mut *guard;
        let impulse = inner
            .clients
            .require(actor)
            .expect("Q1 source client is not admitted")
            .impulse;
        let held = inner.clients.held_input(actor);
        let selected = inner.services.selected_player(actor);
        let grapple_selected = inner.services.selected_weapon(actor).as_deref() == Some("q1:ctf/weapon/grapple");
        CtfInput {
            attack: held.attack,
            jump: held.jump,
            impulse,
            grapple_selected,
            view_angles: selected.view_angles,
            teleport_until: selected.teleport_until,
            frame: selected.frame,
        }
    }

    fn consume_impulse(&mut self, actor: &ActorId) {
        self.lock()
            .clients
            .require_mut(actor)
            .expect("Q1 source client is not admitted")
            .impulse = 0;
    }

    fn observer(&mut self, actor: &ActorId) -> bool {
        self.lock()
            .clients
            .require(actor)
            .expect("Q1 source client is not admitted")
            .observer
    }

    fn set_observer(&mut self, actor: &ActorId, observer: bool) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        inner
            .clients
            .set_observer(&mut *inner.services, actor, observer)
            .expect("Q1 source client is not admitted");
    }

    fn respawn(&mut self, actor: &ActorId, spot: Option<&ActorId>) {
        self.lock().pending.push_back(PendingComposition::Respawn {
            actor: actor.clone(),
            spot: spot.cloned(),
        });
    }

    fn disconnect(&mut self, actor: &ActorId) {
        self.lock()
            .pending
            .push_back(PendingComposition::Disconnect { actor: actor.clone() });
    }

    fn colors(&mut self, actor: &ActorId, shirt: i32, pants: i32) {
        let mut guard = self.lock();
        let inner = &mut *guard;
        inner
            .clients
            .set_colors_data(actor, shirt, pants)
            .expect("Q1 source client is not admitted");
        inner
            .clients
            .publish(&mut *inner.services, actor)
            .expect("Q1 source client is not admitted");
        inner
            .pending
            .push_back(PendingComposition::PlayerSettings { actor: actor.clone() });
    }

    fn prompt_supported(&mut self, actor: &ActorId) -> bool {
        self.lock().services.prompt_supported(actor)
    }

    fn prompt(&mut self, actor: &ActorId, title: &str, choices: &[CtfPromptChoice]) {
        let choices = choices
            .iter()
            .map(|choice| Q1CompositionPromptChoice {
                label: choice.label.clone(),
                impulse: choice.impulse,
            })
            .collect();
        self.lock().services.emit(Q1CompositionEvent::Prompt {
            actor: actor.clone(),
            title: title.to_string(),
            choices,
        });
    }

    fn clear_prompt(&mut self, actor: &ActorId) {
        self.lock()
            .services
            .emit(Q1CompositionEvent::ClearPrompt { actor: actor.clone() });
    }

    fn teleport(&mut self, actor: &ActorId, origin: Vec3, angles: Vec3, velocity: Vec3, until: f64) {
        self.lock().services.teleport(actor, origin, angles, velocity, until);
    }

    fn select_grapple(&mut self, actor: &ActorId) {
        self.lock()
            .services
            .select_weapon(actor, &String::from("q1:ctf/weapon/grapple"));
    }

    fn selected_weapon(&mut self, actor: &ActorId) -> Option<ItemId> {
        self.lock().services.selected_weapon(actor)
    }

    fn selected_ammo(&mut self, actor: &ActorId) -> Option<ItemId> {
        self.lock().services.selected_ammo(actor)
    }

    fn weapon_changed(&mut self, actor: &ActorId, acquired: Option<&ItemId>) {
        self.lock().services.weapon_changed(actor, acquired);
    }

    fn haste(&mut self, actor: &ActorId, _enabled: bool) {
        self.lock().services.weapon_changed(actor, None);
    }

    fn status(&mut self, actor: &ActorId, status: crate::q1::addons::ctf::types::CtfStatus) {
        self.lock().services.emit(Q1CompositionEvent::CtfStatus {
            actor: actor.clone(),
            status,
        });
    }

    fn log(&mut self, actor: &ActorId, action: &str) {
        self.lock().services.emit(Q1CompositionEvent::SourceLog {
            actor: actor.clone(),
            action: action.to_string(),
        });
    }
}

/// Horde services bridge.
struct HordeBridge {
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
}

impl Q1HordeServices for HordeBridge {
    fn dead_flag(&mut self, player: &ActorId) -> i32 {
        lock_inner(&self.shared).services.selected_player(player).dead_flag
    }

    fn no_target(&mut self, player: &ActorId) -> bool {
        lock_inner(&self.shared)
            .clients
            .client(player)
            .is_some_and(|client| client.no_target)
    }

    fn is_bot(&mut self, player: &ActorId) -> bool {
        lock_inner(&self.shared).services.selected_player(player).is_bot
    }

    fn respawn_teammate(&mut self, player: &ActorId) {
        lock_inner(&self.shared).pending.push_back(PendingComposition::Respawn {
            actor: player.clone(),
            spot: None,
        });
    }

    fn add_score(&mut self, player: &ActorId, delta: f64) {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner
            .clients
            .add_score(&mut *inner.services, player, delta)
            .expect("Q1 source client is not admitted");
    }

    fn restart_session(&mut self, map: &str, starting_server_flags: i32) {
        lock_inner(&self.shared)
            .services
            .restart_session(map, starting_server_flags);
    }
}

/// Mission-pack session hooks.
fn mission_pack_options(shared: &Arc<Mutex<Inner>>) -> Q1MissionPackOptions {
    let gamecfg = shared.clone();
    let team_color = shared.clone();
    let set_team_color = shared.clone();
    let add_frags = shared.clone();
    let frags = shared.clone();
    let disconnect = shared.clone();
    let player_frame = shared.clone();
    let player_name = shared.clone();
    let cheat_arsenal = shared.clone();
    let cheats_allowed = shared.clone();
    let developer_message = shared.clone();
    let footsteps = shared.clone();
    Q1MissionPackOptions {
        present_finale: None,
        gamecfg: Some(Arc::new(move || lock_inner(&gamecfg).services.cvar("gamecfg") as i32)),
        team_color: Some(Arc::new(move |actor| {
            lock_inner(&team_color)
                .clients
                .require(actor)
                .map(|client| client.team)
                .expect("Q1 source client is not admitted")
        })),
        set_team_color: Some(Arc::new(move |actor, color| {
            let mut guard = lock_inner(&set_team_color);
            let inner = &mut *guard;
            inner
                .clients
                .set_colors_data(actor, color - 1, color - 1)
                .expect("Q1 source client is not admitted");
            inner
                .clients
                .publish(&mut *inner.services, actor)
                .expect("Q1 source client is not admitted");
            inner
                .pending
                .push_back(PendingComposition::PlayerSettings { actor: actor.clone() });
        })),
        add_frags: Some(Arc::new(move |actor, delta| {
            let mut guard = lock_inner(&add_frags);
            let inner = &mut *guard;
            inner
                .clients
                .add_score(&mut *inner.services, actor, f64::from(delta))
                .expect("Q1 source client is not admitted");
        })),
        frags: Some(Arc::new(move |actor| {
            lock_inner(&frags)
                .clients
                .require(actor)
                .map(|client| client.frags as i32)
                .expect("Q1 source client is not admitted")
        })),
        disconnect: Some(Arc::new(move |actor| {
            lock_inner(&disconnect)
                .pending
                .push_back(PendingComposition::Disconnect { actor: actor.clone() });
        })),
        player_frame: Some(Arc::new(move |actor| {
            lock_inner(&player_frame).services.selected_player(actor).frame
        })),
        player_name: Some(Arc::new(move |actor| {
            lock_inner(&player_name)
                .clients
                .require(actor)
                .map(|client| client.name())
                .expect("Q1 source client is not admitted")
        })),
        cheat_arsenal: Some(Rc::new(move |actor, category| {
            let category = match category {
                CheatArsenalCategory::Weapons => Some(Q1CompositionCheatCategory::Weapons),
                CheatArsenalCategory::Ammo => Some(Q1CompositionCheatCategory::Ammo),
            };
            lock_inner(&cheat_arsenal).services.cheat_arsenal(actor, category)
        })),
        cheats_allowed: Some(Rc::new(move || {
            lock_inner(&cheats_allowed).services.cvar("sv_cheats") != 0.0
        })),
        developer_message: Some(Rc::new(move |text| {
            lock_inner(&developer_message)
                .services
                .emit(Q1CompositionEvent::DeveloperMessage { text: text.to_string() });
        })),
        footsteps: Some(Rc::new(move || {
            lock_inner(&footsteps).services.cvar("footsteps") == 1.0
        })),
    }
}

/// Client record release hook.
struct ClientReleaseHook {
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
}

impl Q1ReleaseHook for ClientReleaseHook {
    fn on_release(&mut self, _game: &mut Q1EntityServices, actor: &OwnedActor) {
        lock_inner(&self.shared).clients.remove(actor.id());
    }
}

/// Source client checkpoint extension (`q1:source-clients`).
struct SourceClientsExtension {
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
    /// Selected program.
    program: Q1SourceProgram,
}

impl Q1StateExtension for SourceClientsExtension {
    fn id(&self) -> &str {
        "q1:source-clients"
    }

    fn capture(&self, _game: &Q1EntityServices) -> Vec<u8> {
        lock_inner(&self.shared).clients.capture(self.program)
    }

    fn restore(&mut self, game: &mut Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
        lock_inner(&self.shared).clients.restore(game, self.program, bytes)
    }
}

/// Campaign binding forwarding to the shared selection campaign. Skill
/// changes also write the `skill` variable, like the donor.
struct CompositionCampaign {
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
}

impl Q1CampaignBinding for CompositionCampaign {
    fn read_flags(&self) -> i32 {
        lock_inner(&self.shared).campaign.read_flags()
    }

    fn write_flags(&mut self, flags: i32) {
        lock_inner(&self.shared).campaign.write_flags(flags);
    }

    fn set_skill(&mut self, skill: i32) {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner.campaign.set_skill(skill);
        inner.services.set_cvar("skill", &skill.to_string());
    }
}

/// Grapple selection snapshot across CTF registration. Registration
/// only reads the selection and the native-slot probe; snapshotting
/// avoids holding the state lock across registration, which reenters
/// session hooks through policy sync.
struct SnapshotGrapple {
    /// Snapshotted selection.
    selection: GrappleSelection,
    /// Snapshotted Threewave native slot.
    native_threewave: bool,
}

impl SharedGrappleControl for SnapshotGrapple {
    fn selection(&self) -> &GrappleSelection {
        &self.selection
    }

    fn native_slot(&self, mechanic: GrappleMechanic) -> bool {
        mechanic == GrappleMechanic::Q1Threewave && self.native_threewave
    }

    fn input(&self, _actor: &ActorId, _held: bool) {}

    fn release(&self, _actor: &ActorId) {}

    fn pulling(&self, _actor: &ActorId) -> bool {
        false
    }

    fn gravity_scale(&self, _actor: &ActorId) -> u8 {
        0
    }
}

/// Weapon game resolution for impulses.
enum Arsenal<'game> {
    /// Main game.
    Main,
    /// Foreign weapon game.
    Foreign(&'game mut Q1EntityServices),
    /// Foreign arsenal without a game: skip weapon logic.
    Absent,
}

/// Registers one official source program on the session's existing
/// world and actors (`Q1SourceComposition`). The game must stay at a
/// stable address while the composition is registered.
pub struct Q1SourceComposition {
    /// Selected program.
    program: Q1SourceProgram,
    /// Mission-pack runtime for hipnotic/rogue.
    packs: Option<Q1MissionPackRuntime>,
    /// Addon context for dopa/mg1/mg3/ctf.
    addon: Option<Q1AddonContext>,
    /// Shared state.
    shared: Arc<Mutex<Inner>>,
}

impl Q1SourceComposition {
    /// Register a source composition on a game.
    pub fn register(
        game: &mut Q1EntityServices,
        selection: Q1SourceSelection,
        services: Box<dyn Q1CompositionServices>,
    ) -> Result<Self, Q1Error> {
        let program = selection.program;
        let shared = Arc::new(Mutex::new(Inner {
            clients: Q1SourceClients::new(),
            services,
            pending: VecDeque::new(),
            campaign: selection.campaign,
            finale: FinaleAcknowledgement::new(),
        }));
        game.register_release_hook(Rc::new(RefCell::new(ClientReleaseHook { shared: shared.clone() })));
        if game.options().edition == Q1Edition::Rerelease
            && matches!(
                program,
                Q1SourceProgram::Id1 | Q1SourceProgram::Hipnotic | Q1SourceProgram::Rogue
            )
        {
            let bots = shared.clone();
            let coop = game.options().coop;
            game.register_damage_source_effects(
                "q1:coop-bots",
                Q1DamageSourceEffects {
                    before_quad: Some(Box::new(move |request, amount, _, _| {
                        let attacker = request.attack.attacker.clone();
                        let cancel =
                            coop && attacker.as_ref().is_some_and(|attacker| {
                                let inner = lock_inner(&bots);
                                !same_actor(&request.target, attacker)
                                    && inner.clients.client(&request.target).is_some()
                                    && inner.clients.client(attacker).is_some()
                            }) && attacker
                                .as_ref()
                                .is_some_and(|attacker| lock_inner(&bots).services.selected_player(attacker).is_bot)
                                && !lock_inner(&bots).services.selected_player(&request.target).is_bot;
                        if cancel {
                            DamagePreparation::Cancel
                        } else {
                            DamagePreparation::Continue { amount }
                        }
                    })),
                    ..Default::default()
                },
            )?;
        }
        if program == Q1SourceProgram::Rogue {
            game.set_base_team_health();
        }
        register_q1_base(
            game,
            Q1BaseOptions {
                campaign: Some(Box::new(CompositionCampaign { shared: shared.clone() })),
                registered: Some(selection.registered),
                official_campaign: Some(selection.official_campaign),
                ..Default::default()
            },
        )?;
        let (packs, addon) = match program {
            Q1SourceProgram::Hipnotic | Q1SourceProgram::Rogue => {
                let pack = if program == Q1SourceProgram::Hipnotic {
                    Q1MissionPack::Hipnotic
                } else {
                    Q1MissionPack::Rogue
                };
                let packs = register_q1_mission_pack(game, pack, mission_pack_options(&shared))?;
                (Some(packs), None)
            }
            Q1SourceProgram::Ctf => {
                let context = register_addon_context(
                    game,
                    Q1AddonProgram::Ctf,
                    Box::new(AddonBridge { shared: shared.clone() }),
                )?;
                let snapshot = lock_inner(&shared)
                    .services
                    .shared_grapple()
                    .map(|control| SnapshotGrapple {
                        selection: control.selection().clone(),
                        native_threewave: control.native_slot(GrappleMechanic::Q1Threewave),
                    });
                register_ctf(
                    game,
                    Box::new(CtfBridge { shared: shared.clone() }),
                    snapshot.as_ref().map(|snapshot| snapshot as &dyn SharedGrappleControl),
                )?;
                (None, Some(context))
            }
            Q1SourceProgram::Dopa | Q1SourceProgram::Mg1 | Q1SourceProgram::Mg3 => {
                let addon_program = match program {
                    Q1SourceProgram::Dopa => Q1AddonProgram::Dopa,
                    Q1SourceProgram::Mg1 => Q1AddonProgram::Mg1,
                    _ => Q1AddonProgram::Mg3,
                };
                let context =
                    register_q1_campaign_addons(game, addon_program, Box::new(AddonBridge { shared: shared.clone() }))?;
                if matches!(program, Q1SourceProgram::Mg1 | Q1SourceProgram::Dopa) {
                    register_q1_horde(game, Box::new(HordeBridge { shared: shared.clone() }))?;
                }
                (None, Some(context))
            }
            Q1SourceProgram::Id1 => (None, None),
        };
        if matches!(addon.as_ref().map(Q1AddonContext::program), Some(Q1AddonProgram::Mg3)) {
            let key = addon_key(game);
            game.register_damage_source_effects(
                "q1:mg3:buddha",
                Q1DamageSourceEffects {
                    lethal_health: Some(Box::new(move |request, health, _, _| {
                        if addon_player_number_by_key(key, &request.target, "buddha") != 0.0 {
                            Q1LethalHealth {
                                health: 1.0,
                                reaction: Q1LethalReaction::None,
                            }
                        } else {
                            Q1LethalHealth {
                                health,
                                reaction: Q1LethalReaction::Death,
                            }
                        }
                    })),
                    ..Default::default()
                },
            )?;
        }
        game.register_state_extension(Box::new(SourceClientsExtension {
            shared: shared.clone(),
            program,
        }))?;
        Ok(Self {
            program,
            packs,
            addon,
            shared,
        })
    }

    /// Selected program.
    #[must_use]
    pub fn program(&self) -> Q1SourceProgram {
        self.program
    }

    /// Admit a client (`attach`).
    pub fn attach(
        &self,
        game: &mut Q1EntityServices,
        actor: &OwnedActor,
        admission: &Q1ClientAdmission,
    ) -> Result<(), Q1Error> {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner
            .clients
            .attach(game, &mut *inner.services, self.program, actor, admission)?;
        drop(guard);
        self.notice(game, actor.id(), Q1ClientEvent::Connect)?;
        let same = lock_inner(&self.shared).services.cvar("samelevel") != 0.0;
        let result = level_client_connected(game, game.time, same)?;
        self.present_intermission(game, &result)?;
        self.drain_pending(game)
    }

    /// Note a spawn after the outer session initialized the selected
    /// character, arsenal, and travel (`spawned`).
    pub fn spawned(&self, game: &mut Q1EntityServices, actor: &ActorId, first_admission: bool) -> Result<(), Q1Error> {
        let mut guard = lock_inner(&self.shared);
        let client = guard.clients.require_mut(actor)?;
        client.death_recorded = false;
        client.impulse = 0;
        client.use_action = false;
        client.respawn_requested_at = -1.0;
        drop(guard);
        if !first_admission {
            level_reset_player(game, actor)?;
            if matches!(self.program, Q1SourceProgram::Mg1 | Q1SourceProgram::Dopa) {
                horde_restore_keys(game, actor)?;
            }
        }
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner.clients.spawned(game, &mut *inner.services, self.program, actor)?;
        drop(guard);
        if let Some(packs) = &self.packs {
            packs.player_spawned(game, actor)?;
        }
        if self.program == Q1SourceProgram::Ctf {
            ctf_spawn_player(game, actor, first_admission)?;
        }
        self.drain_pending(game)
    }

    /// Replace userinfo (`userinfo`).
    pub fn userinfo(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        values: &[(String, String)],
    ) -> Result<(), Q1Error> {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner
            .clients
            .update(game, &mut *inner.services, self.program, actor, values)?;
        drop(guard);
        self.drain_pending(game)
    }

    /// Replace a client userinfo map without side effects (donor `storePlayerUserinfo` q1 arm).
    pub fn replace_userinfo(&self, actor: &ActorId, values: &[(String, String)]) -> Result<(), Q1Error> {
        lock_inner(&self.shared).clients.require_mut(actor)?.userinfo = values.to_vec();
        Ok(())
    }

    /// Admitted client snapshot (`clients.require`, cloned).
    #[must_use]
    pub fn client(&self, actor: &ActorId) -> Option<Q1SourceClient> {
        lock_inner(&self.shared).clients.client(actor).cloned()
    }

    /// Set shirt and pants colors (`clients.colors`).
    pub fn colors(&self, game: &mut Q1EntityServices, actor: &ActorId, shirt: i32, pants: i32) -> Result<(), Q1Error> {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner
            .clients
            .colors(game, &mut *inner.services, self.program, actor, shirt, pants)?;
        drop(guard);
        self.drain_pending(game)
    }

    /// Replace frags and publish (`clients.require` + `publish`).
    pub fn set_frags(&self, game: &mut Q1EntityServices, actor: &ActorId, frags: f64) -> Result<(), Q1Error> {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner.clients.require_mut(actor)?.frags = frags;
        inner.clients.publish(&mut *inner.services, actor)?;
        drop(guard);
        self.drain_pending(game)
    }

    /// Whether the actor is untargetable (`noTarget`).
    #[must_use]
    pub fn no_target(&self, actor: &ActorId) -> bool {
        lock_inner(&self.shared)
            .clients
            .client(actor)
            .is_some_and(|client| client.no_target)
    }

    /// Set untargetable mode (`setNoTarget`).
    pub fn set_no_target(&self, actor: &ActorId, enabled: bool) -> Result<(), Q1Error> {
        let mut guard = lock_inner(&self.shared);
        let inner = &mut *guard;
        inner.clients.require_mut(actor)?.no_target = enabled;
        inner.clients.publish(&mut *inner.services, actor)
    }

    /// Apply source input (`input`).
    pub fn input(&self, game: &mut Q1EntityServices, actor: &ActorId, input: &Q1SourceInput) -> Result<(), Q1Error> {
        let owned = {
            let mut guard = lock_inner(&self.shared);
            let owned = {
                let client = guard.clients.require_mut(actor)?;
                if input.impulse != 0 {
                    client.impulse = input.impulse;
                }
                client.use_action = input.use_action;
                client.actor.clone()
            };
            guard.clients.record_input(actor, input.attack, input.jump);
            owned
        };
        let teleport_until = lock_inner(&self.shared).services.selected_player(actor).teleport_until;
        game.player_input(
            &owned,
            &Q1PlayerInput {
                attack: input.attack,
                jump: input.jump,
                teleport_until: Some(teleport_until),
            },
        );
        self.drain_pending(game)
    }

    /// Frame phase after `beginFrame`, before source player prethink
    /// (`preFrame`).
    pub fn pre_frame(&self, game: &mut Q1EntityServices, elapsed_seconds: f64) -> Result<(), Q1Error> {
        if self.addon.is_some() {
            frame_addons(game, elapsed_seconds)?;
        }
        if game.options().deathmatch != 0 {
            let (scores, timelimit, fraglimit) = {
                let mut inner = lock_inner(&self.shared);
                let scores: Vec<f64> = inner.clients.records.values().map(|client| client.frags).collect();
                let timelimit = inner.services.cvar("timelimit");
                let fraglimit = inner.services.cvar("fraglimit");
                (scores, timelimit, fraglimit)
            };
            level_check_limits(game, game.time, &scores, timelimit, fraglimit)?;
        }
        self.poll_finale(game)?;
        self.drain_pending(game)
    }

    /// Poll the finale acknowledgement from the frame phase. Dismissal
    /// is only read while a finale is active and every finale resets
    /// it first, so polling outside finales is harmless.
    fn poll_finale(&self, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        let actors: Vec<ActorId> = lock_inner(&self.shared).clients.records.keys().cloned().collect();
        let mut buttons = HashMap::new();
        for actor in &actors {
            buttons.insert(
                actor.clone(),
                game.player_ref(actor).is_some_and(|player| player.attack_held),
            );
        }
        if lock_inner(&self.shared).finale.poll(game.time, &buttons) {
            dismiss_finale(game)?;
        }
        Ok(())
    }

    /// Source prethink before `playerFrame` (`playerPreThink`).
    pub fn player_pre_think(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            ctf_player_frame(game, actor)?;
        }
        self.drain_pending(game)
    }

    /// Source postthink after selected movement and
    /// `playerAfterPhysics` (`playerPostThink`).
    pub fn player_post_think(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            ctf_after_physics(game, actor)?;
        }
        if self.addon.is_some() {
            let view_offset = lock_inner(&self.shared).services.selected_player(actor).view_offset;
            frame_q1_addon_player(game, actor, view_offset)?;
        }
        self.drain_pending(game)
    }

    /// Handle a pending impulse. Returns true only when the selected
    /// source program consumed it (`impulse`).
    pub fn impulse(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
        let result = self.impulse_inner(game, actor)?;
        self.drain_pending(game)?;
        Ok(result)
    }

    /// Impulse gates without the drain.
    fn impulse_inner(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<bool, Q1Error> {
        let pending = lock_inner(&self.shared).clients.require(actor)?.impulse;
        if pending == 0 {
            return Ok(false);
        }
        let foreign = lock_inner(&self.shared).services.weapon_services(actor);
        // None selects the main game; a null pointer skips weapon
        // logic; otherwise the foreign game owns the arsenal.
        let arsenal = match foreign {
            None => Arsenal::Main,
            Some(pointer) if pointer.is_null() => Arsenal::Absent,
            // SAFETY: the services contract keeps a returned game
            // disjoint from the calling game and valid for the entry;
            // the pointer is dereferenced only here, while no other
            // borrow of the foreign game exists.
            Some(pointer) => Arsenal::Foreign(unsafe { &mut *pointer }),
        };
        let gated = match &arsenal {
            Arsenal::Main => game
                .player_ref(actor)
                .is_some_and(|player| game.time < player.attack_finished),
            Arsenal::Foreign(weapons) => weapons
                .player_ref(actor)
                .is_some_and(|player| weapons.time < player.attack_finished),
            Arsenal::Absent => false,
        };
        if gated {
            return Ok(false);
        }
        if self.program == Q1SourceProgram::Ctf && ctf_impulse(game, actor)? {
            return Ok(true);
        }
        if let Some(packs) = &self.packs {
            if packs.impulse(game, actor, pending)? {
                self.clear_impulse(actor)?;
                return Ok(true);
            }
        }
        if self.addon.is_some() {
            let shared = self.shared.clone();
            let mut developer = |text: &str| {
                lock_inner(&shared)
                    .services
                    .emit(Q1CompositionEvent::DeveloperMessage { text: text.to_string() });
            };
            if handle_mg3_item_impulse(game, actor, pending, &mut developer)? {
                self.clear_impulse(actor)?;
                return Ok(true);
            }
            if handle_q1_addon_impulse(game, actor, pending)? {
                self.clear_impulse(actor)?;
                return Ok(true);
            }
        }
        match arsenal {
            Arsenal::Main => {
                if let Some(player) = game.player_ref(actor).cloned() {
                    if q1_weapon_impulse(game, &player, pending)? {
                        self.clear_impulse(actor)?;
                        return Ok(true);
                    }
                }
            }
            Arsenal::Foreign(weapons) => {
                if let Some(player) = weapons.player_ref(actor).cloned() {
                    if q1_weapon_impulse(weapons, &player, pending)? {
                        self.clear_impulse(actor)?;
                        return Ok(true);
                    }
                }
            }
            Arsenal::Absent => {}
        }
        let Some(player) = game.player_ref(actor).cloned() else {
            return Ok(false);
        };
        let mut guard = lock_inner(&self.shared);
        base_q1_impulse(game, &mut *guard.services, self.program, &player, pending)?;
        drop(guard);
        self.clear_impulse(actor)?;
        Ok(true)
    }

    /// Clear a pending impulse.
    fn clear_impulse(&self, actor: &ActorId) -> Result<(), Q1Error> {
        lock_inner(&self.shared).clients.require_mut(actor)?.impulse = 0;
        Ok(())
    }

    /// Note an accepted shot (`fired`).
    pub fn fired(&self, game: &mut Q1EntityServices, actor: &ActorId, weapon: Option<&ItemId>) -> Result<(), Q1Error> {
        if lock_inner(&self.shared).clients.client(actor).is_none() {
            return Ok(());
        }
        level_note_attack(game, actor, weapon.is_some_and(|weapon| weapon == "q1:weapon/axe"))?;
        self.drain_pending(game)
    }

    /// Death and damage reaction hook inside
    /// `GameplayAuthority.beforeReaction` (`beforeReaction`).
    pub fn before_reaction(
        &self,
        game: &mut Q1EntityServices,
        actor: &OwnedActor,
        decision: &DamageDecision,
    ) -> Result<(), Q1Error> {
        let id = actor.id().clone();
        let has_client = lock_inner(&self.shared).clients.client(&id).is_some();
        if decision.reaction == DamageReaction::Death
            && (has_client || game.entity_ref(&id).is_some())
            && game.health(&id) < -99.0
        {
            game.host.combat.set_health(actor, -99.0)?;
        }
        if !has_client {
            let Some(entity) = game.entity_ref(&id).cloned() else {
                return Ok(());
            };
            if decision.reaction != DamageReaction::Death
                || matches!(entity.movement, Q1MoveType::None | Q1MoveType::Push)
            {
                return Ok(());
            }
            let attacker = decision.request.attack.attacker.clone();
            if game.options().edition == Q1Edition::Rerelease
                && entity.movement_flags & 32 != 0
                && entity.text("horde.sourceDie").is_empty()
            {
                if let Some(attacker) = attacker {
                    if lock_inner(&self.shared).clients.client(&attacker).is_some() {
                        let mut guard = lock_inner(&self.shared);
                        let inner = &mut *guard;
                        inner.clients.add_score(&mut *inner.services, &attacker, 1.0)?;
                    }
                }
            }
            // ClientObituary consumes its source rnum even when the
            // victim is not a player.
            game.host.random();
            return Ok(());
        }
        level_note_damage(game, &id, decision.applied_damage)?;
        if decision.applied_damage > 0.0 {
            if let Some(packs) = &self.packs {
                packs.confirmed_damage(game, &id, decision.request.attack.attacker.as_ref())?;
            }
        }
        if decision.reaction != DamageReaction::Death || lock_inner(&self.shared).clients.require(&id)?.death_recorded {
            return Ok(());
        }
        lock_inner(&self.shared).clients.require_mut(&id)?.death_recorded = true;
        let attacker = decision.request.attack.attacker.clone();
        let source_attacker = attacker
            .as_ref()
            .and_then(|attacker| game.entity_ref(attacker).cloned());
        let death_type = match &decision.request.attack.cause {
            AttackCause::Q1 { death_type, .. } => death_type.clone(),
            AttackCause::Environment { hazard } if *hazard == EnvironmentHazard::Fall => String::from("falling"),
            _ => String::new(),
        };
        let teamplay = lock_inner(&self.shared).services.cvar("teamplay") as i32;
        let victim = self.obituary_actor(game, &id)?;
        let obituary_attacker = attacker
            .as_ref()
            .map(|attacker| self.obituary_actor(game, attacker))
            .transpose()?;
        let telefrag_owner = source_attacker
            .as_ref()
            .and_then(|entity| entity.owner.clone())
            .map(|owner| self.obituary_actor(game, &owner))
            .transpose()?;
        let input = Q1ObituaryInput {
            edition: game.options().edition,
            victim,
            attacker: obituary_attacker,
            telefrag_owner,
            teamplay,
            death_type,
        };
        let obituary = match &self.packs {
            Some(packs) => packs.obituary(game, &input, decision.request.attack.inflictor.as_ref()),
            None => q1_obituary(&input, &mut || game.host.random()),
        };
        if let Some(message) = &obituary.message {
            self.broadcast(game, &message.text, &message.arguments);
        }
        if self.program == Q1SourceProgram::Ctf {
            ctf_death(game, &id, attacker.as_ref())?;
        } else {
            if let Some(score) = &obituary.score {
                if lock_inner(&self.shared).clients.client(&score.actor).is_some() {
                    let mut guard = lock_inner(&self.shared);
                    let inner = &mut *guard;
                    inner
                        .clients
                        .add_score(&mut *inner.services, &score.actor, f64::from(score.delta))?;
                }
            }
            if matches!(self.program, Q1SourceProgram::Mg1 | Q1SourceProgram::Dopa) {
                if let Some(attacker) = &attacker {
                    if lock_inner(&self.shared).clients.client(attacker).is_some() {
                        horde_teammate_killed(game, attacker)?;
                    }
                }
            }
        }
        if let Some(achievement) = &obituary.achievement {
            game.host.emit(Q1Event::Achievement {
                player: Some(achievement.actor.clone()),
                id: achievement.id.clone(),
            });
        }
        if let Some(packs) = &self.packs {
            packs.player_died(game, &id, attacker.as_ref())?;
        }
        self.drain_pending(game)
    }

    /// Build an obituary participant (`obituaryActor`).
    fn obituary_actor(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<Q1ObituaryActor, Q1Error> {
        let (client, selected) = {
            let mut inner = lock_inner(&self.shared);
            let client = inner.clients.client(actor).cloned();
            let selected = client.as_ref().map(|_| inner.services.selected_player(actor));
            (client, selected)
        };
        let entity = game.entity_ref(actor).cloned();
        let player = game.player_ref(actor).cloned();
        let name = client.as_ref().map_or_else(
            || entity.as_ref().map(|entity| entity.text("netname")).unwrap_or_default(),
            |client| client.name(),
        );
        let classname = if client.is_none() {
            game.host.classname(actor)
        } else {
            String::from("player")
        };
        Ok(Q1ObituaryActor {
            actor: actor.clone(),
            name,
            classname,
            is_player: client.is_some(),
            is_monster: entity.as_ref().is_some_and(|entity| entity.movement_flags & 32 != 0),
            team: client.as_ref().map_or_else(
                || entity.as_ref().map(|entity| entity.number("team") as i32).unwrap_or(0),
                |client| client.team,
            ),
            health: game.health(actor),
            water_type: selected
                .as_ref()
                .map_or(Q1DeathWater::Empty, |selected| selected.water_type),
            water_level: selected.as_ref().map_or_else(
                || entity.as_ref().map(|entity| entity.water_level).unwrap_or(0),
                |selected| selected.water_level,
            ),
            weapon: player.as_ref().map(|player| player.weapon),
            quad_expires: (player
                .as_ref()
                .and_then(|player| player.powerups.get(&Q1Powerup::Quad).copied())
                .unwrap_or(0.0)
                - game.time)
                .max(0.0),
            invulnerable_expires: (player
                .as_ref()
                .and_then(|player| player.powerups.get(&Q1Powerup::Invulnerability).copied())
                .unwrap_or(0.0)
                - game.time)
                .max(0.0),
            brush: entity.as_ref().is_some_and(|entity| entity.solid == Q1Solid::Bsp),
            kill_string: entity
                .as_ref()
                .map(|entity| entity.text("kill_string"))
                .unwrap_or_default(),
        })
    }

    /// Broadcast a message to every admitted player (`broadcast`).
    fn broadcast(&self, game: &mut Q1EntityServices, text: &str, args: &[String]) {
        for actor in (game.host.players)() {
            let args: Vec<Q1MessageArg> = args.iter().map(|arg| Q1MessageArg::Text(arg.clone())).collect();
            game.message(Some(&actor), text, false, args);
        }
    }

    /// Client lifecycle notice (`notice`).
    fn notice(&self, game: &mut Q1EntityServices, actor: &ActorId, event: Q1ClientEvent) -> Result<(), Q1Error> {
        let (name, frags) = {
            let inner = lock_inner(&self.shared);
            let client = inner.clients.require(actor)?;
            (client.name(), client.frags)
        };
        let notice = q1_client_notice(game.options().edition, event, &name, frags as i32);
        self.broadcast(game, &notice.text, &notice.arguments);
        if notice.score_delta != 0 {
            let mut guard = lock_inner(&self.shared);
            let inner = &mut *guard;
            inner
                .clients
                .add_score(&mut *inner.services, actor, f64::from(notice.score_delta))?;
        }
        Ok(())
    }

    /// Disconnect a client (`disconnect`).
    pub fn disconnect(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        self.disconnect_inner(game, actor)?;
        self.drain_pending(game)
    }

    /// Disconnect without the drain.
    fn disconnect_inner(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        let slot = lock_inner(&self.shared).clients.require(actor)?.slot;
        if self.program == Q1SourceProgram::Ctf {
            ctf_disconnect_player(game, actor)?;
        }
        if let Some(packs) = &self.packs {
            packs.player_died(game, actor, None)?;
        }
        self.notice(game, actor, Q1ClientEvent::Disconnect)?;
        let mut inner = lock_inner(&self.shared);
        inner.services.emit(Q1CompositionEvent::ClientLeft {
            actor: actor.clone(),
            slot,
        });
        inner.services.disconnect(actor);
        Ok(())
    }

    /// Request a respawn (`requestRespawn`).
    pub fn request_respawn(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        coop_entry: Option<&Q1TravelState>,
    ) -> Result<(), Q1Error> {
        if matches!(self.program, Q1SourceProgram::Mg1 | Q1SourceProgram::Dopa) && horde_request_respawn(game)? {
            return self.drain_pending(game);
        }
        if !game.options().coop && game.options().deathmatch == 0 {
            let flags = campaign_read_flags(game)?;
            let map = game.map_name.clone();
            lock_inner(&self.shared).services.restart_session(&map, flags);
            return self.drain_pending(game);
        }
        let travel = match (game.options().coop, coop_entry) {
            (true, Some(entry)) => entry.clone(),
            _ => self.new_travel(game)?,
        };
        self.respawn_at(game, actor, None, &travel)?;
        self.drain_pending(game)
    }

    /// Respawn at a spot (`respawnAt`).
    fn respawn_at(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        source_spot: Option<&ActorId>,
        travel: &Q1TravelState,
    ) -> Result<(), Q1Error> {
        let (owned, requested_at) = {
            let mut inner = lock_inner(&self.shared);
            let client = inner.clients.require_mut(actor)?;
            if client.respawn_requested_at < 0.0 {
                client.respawn_requested_at = game.time;
            }
            (client.actor.clone(), client.respawn_requested_at)
        };
        let spot = match source_spot {
            Some(spot) => Some(spot.clone()),
            None => self.select_spawn(game, Some(actor), game.time >= requested_at + 5.0)?,
        };
        let Some(spot) = spot else {
            return Ok(());
        };
        let entity = game
            .entity_ref(&spot)
            .cloned()
            .ok_or_else(|| q1_error("Q1 spawn point has no entity"))?;
        lock_inner(&self.shared).services.place_player(&owned, &entity, travel);
        self.spawned(game, actor, false)
    }

    /// Select a spawn point (`selectSpawn`).
    pub fn select_spawn(
        &self,
        game: &mut Q1EntityServices,
        actor: Option<&ActorId>,
        force: bool,
    ) -> Result<Option<ActorId>, Q1Error> {
        if let Some(actor) = actor {
            if self.program == Q1SourceProgram::Ctf {
                return ctf_select_spawn(game, actor);
            }
            if let Some(packs) = &self.packs {
                if let Some(point) = packs.select_spawn(game, actor) {
                    return Ok(Some(point));
                }
            }
        }
        spawn_select(game, force)
    }

    /// Fresh travel state (`newTravel`).
    pub fn new_travel(&self, game: &mut Q1EntityServices) -> Result<Q1TravelState, Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            return new_q1_ctf_travel(game);
        }
        if let Some(packs) = &self.packs {
            return Ok(packs.new_travel(game));
        }
        if let Some(addon) = &self.addon {
            return Ok(new_q1_addon_travel(addon, game));
        }
        Ok(new_q1_travel(game.options()))
    }

    /// Capture travel state (`captureTravel`).
    pub fn capture_travel(&self, game: &mut Q1EntityServices, actor: &OwnedActor) -> Result<Q1TravelState, Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            return capture_q1_ctf_travel(game, actor);
        }
        if let Some(packs) = &self.packs {
            return packs.capture_travel(game, actor);
        }
        if let Some(addon) = &self.addon {
            return capture_q1_addon_travel(addon, game, actor);
        }
        let weapon = self.selected_base_weapon(game, actor.id());
        capture_q1_travel(game, actor, weapon, None, true)
    }

    /// Decode travel state (`decodeTravel`).
    pub fn decode_travel(&self, game: &mut Q1EntityServices, state: &Q1TravelState) -> Result<Q1TravelState, Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            return decode_q1_ctf_travel(game, state);
        }
        if let Some(packs) = &self.packs {
            let flags = campaign_read_flags(game)?;
            return Ok(packs.decode_travel(game, state, flags));
        }
        if let Some(addon) = &self.addon {
            return decode_q1_addon_travel(addon, game, state);
        }
        let flags = campaign_read_flags(game)?;
        Ok(decode_q1_travel(game, state, flags))
    }

    /// Admit travel state (`admitTravel`).
    pub fn admit_travel(
        &self,
        game: &mut Q1EntityServices,
        actor: &OwnedActor,
        state: &Q1TravelState,
    ) -> Result<(), Q1Error> {
        if let Some(packs) = &self.packs {
            let flags = campaign_read_flags(game)?;
            let decoded = packs.decode_travel(game, state, flags);
            return packs.admit_travel(game, actor, &decoded);
        }
        if let Some(addon) = &self.addon {
            if self.program != Q1SourceProgram::Ctf {
                return admit_q1_addon_travel(addon, game, actor, state);
            }
        }
        let decoded = self.decode_travel(game, state)?;
        admit_q1_travel(game, actor, &decoded)
    }

    /// Selected base weapon by selected item, falling back to the
    /// player weapon.
    fn selected_base_weapon(&self, game: &Q1EntityServices, actor: &ActorId) -> Option<Q1Weapon> {
        let selected = lock_inner(&self.shared).services.selected_weapon(actor);
        if let Some(selected) = selected {
            for base in WEAPONS {
                let weapon = Q1Weapon::from(base);
                if game.weapon_item(weapon) == selected {
                    return Some(weapon);
                }
            }
        }
        game.player_ref(actor).map(|player| player.weapon)
    }

    /// Death inventory drop (`dropInventory`).
    pub fn drop_inventory(&self, game: &mut Q1EntityServices, actor: &OwnedActor) -> Result<Option<ActorId>, Q1Error> {
        if !game.options().coop && game.options().deathmatch == 0 {
            return Ok(None);
        }
        if let Some(packs) = &self.packs {
            return packs.drop_backpack(game, actor);
        }
        let body = game
            .host
            .bodies
            .read(actor.id())
            .ok_or_else(|| q1_error("Q1 death drop has no shared body"))?;
        drop_backpack(
            game,
            body.origin,
            &BackpackDrop {
                weapon: self.selected_base_weapon(game, actor.id()),
                shells: game.host.inventory.count(actor.id(), &String::from("q1:ammo/shells")),
                nails: game.host.inventory.count(actor.id(), &String::from("q1:ammo/nails")),
                rockets: game.host.inventory.count(actor.id(), &String::from("q1:ammo/rockets")),
                cells: game.host.inventory.count(actor.id(), &String::from("q1:ammo/cells")),
                ..Default::default()
            },
            None,
        )
    }

    /// Request an intermission exit (`requestIntermissionExit`).
    pub fn request_intermission_exit(
        &self,
        game: &mut Q1EntityServices,
        pressed: bool,
        input: Option<(&ActorId, bool)>,
    ) -> Result<Q1IntermissionResult, Q1Error> {
        if let Some((actor, attack)) = input {
            let owned = lock_inner(&self.shared).clients.require(actor)?.actor.id().clone();
            if game.player_ref(&owned).is_some() {
                game.update_player(&owned, |player| player.attack_held = attack)?;
                let mut inner = lock_inner(&self.shared);
                let jump = inner.clients.held_input(actor).jump;
                inner.clients.record_input(actor, attack, jump);
            }
        }
        let same = lock_inner(&self.shared).services.cvar("samelevel") != 0.0;
        let result = level_request_exit(game, game.time, pressed, same)?;
        self.present_intermission(game, &result)?;
        self.drain_pending(game)?;
        Ok(result)
    }

    /// Present an intermission result (`presentIntermission`).
    fn present_intermission(&self, game: &mut Q1EntityServices, result: &Q1IntermissionResult) -> Result<(), Q1Error> {
        match result {
            Q1IntermissionResult::Finale { text, track } => {
                reset_finale(game)?;
                lock_inner(&self.shared).finale.reset();
                lock_inner(&self.shared)
                    .services
                    .emit(Q1CompositionEvent::LevelPresentation(Q1SourceFinale::Finale {
                        text: text.clone(),
                        track: *track,
                    }));
            }
            Q1IntermissionResult::SellScreen => {
                lock_inner(&self.shared)
                    .services
                    .emit(Q1CompositionEvent::LevelPresentation(Q1SourceFinale::SellScreen));
            }
            Q1IntermissionResult::Waiting | Q1IntermissionResult::Travel { .. } => {}
        }
        Ok(())
    }

    /// Dismiss the finale (`dismissFinale`).
    pub fn dismiss_finale(&self, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        lock_inner(&self.shared).finale.dismiss(game.time);
        dismiss_finale(game)
    }

    /// Character pose (`characterPose`).
    pub fn character_pose(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
    ) -> Result<Q1CharacterSourcePose, Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            let pose = ctf_character_pose(game, actor)?;
            return Ok(Q1CharacterSourcePose {
                axe_pose: Some(pose.axe_pose),
                definition: None,
                frame: pose.frame.map(f64::from),
            });
        }
        if matches!(&self.addon, Some(addon) if addon.program() == Q1AddonProgram::Mg3)
            && game.player_ref(actor).is_some()
        {
            return Ok(Q1CharacterSourcePose {
                axe_pose: None,
                definition: None,
                frame: mg3_hammer_body_frame(game, actor)?.map(f64::from),
            });
        }
        if let Some(packs) = &self.packs {
            return Ok(packs.character_pose(game, actor));
        }
        Ok(Q1CharacterSourcePose::default())
    }

    /// Whether falling damage applies (`fallDamageAllowed`).
    #[must_use]
    pub fn fall_damage_allowed(&self, game: &Q1EntityServices, actor: &ActorId) -> bool {
        if self.program == Q1SourceProgram::Ctf {
            ctf_fall_damage_allowed(game, actor)
        } else {
            true
        }
    }

    /// Character frame effects (`characterFrame`).
    pub fn character_frame(
        &self,
        game: &mut Q1EntityServices,
        actor: &ActorId,
        presentation: &Q1CharacterPresentation,
    ) -> Result<(), Q1Error> {
        if let Some(packs) = &self.packs {
            packs.character_frame(game, actor, presentation)?;
        }
        self.drain_pending(game)
    }

    /// Suicide (`suicide`).
    pub fn suicide(&self, game: &mut Q1EntityServices, actor: &ActorId) -> Result<(), Q1Error> {
        if self.program == Q1SourceProgram::Ctf {
            ctf_suicide(game, actor)?;
        } else {
            self.notice(game, actor, Q1ClientEvent::Suicide)?;
            self.request_respawn(game, actor, None)?;
        }
        self.drain_pending(game)
    }

    /// Spawn a map (`spawnMap`).
    pub fn spawn_map(&self, game: &mut Q1EntityServices, map: &Q1Map<'_>) -> Result<Q1SpawnReport, Q1Error> {
        let file = map.source.rsplit(['/', '\\']).next().unwrap_or(map.source.as_str());
        let name = file.strip_suffix(".bsp").unwrap_or(file);
        lock_inner(&self.shared)
            .services
            .set_cvar("sv_gravity", if name == "e1m8" { "100" } else { "800" });
        let report = game.spawn_map(map)?;
        self.drain_pending(game)?;
        Ok(report)
    }

    /// Spawn the map actor list hook helper.
    fn drain_pending(&self, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        loop {
            let action = lock_inner(&self.shared).pending.pop_front();
            let Some(action) = action else {
                break;
            };
            match action {
                PendingComposition::Respawn { actor, spot } => {
                    if lock_inner(&self.shared).clients.client(&actor).is_none() {
                        continue;
                    }
                    let travel = self.new_travel(game)?;
                    self.respawn_at(game, &actor, spot.as_ref(), &travel)?;
                }
                PendingComposition::Disconnect { actor } => {
                    if lock_inner(&self.shared).clients.client(&actor).is_some() {
                        self.disconnect_inner(game, &actor)?;
                    }
                }
                PendingComposition::PlayerSettings { actor } => {
                    if lock_inner(&self.shared).clients.client(&actor).is_some() {
                        lock_inner(&self.shared)
                            .clients
                            .apply_player_settings(game, self.program, &actor)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Read a client record for tests.
    #[cfg(test)]
    pub(crate) fn test_client(&self, actor: &ActorId) -> Option<Q1SourceClient> {
        lock_inner(&self.shared).clients.client(actor).cloned()
    }
}

/// Create a source composition on a fresh game
/// (`createQ1SourceComposition`). The returned game must stay at a
/// stable address while the composition is registered.
pub fn create_q1_source_composition(
    host: Q1FoundationHost,
    options: Q1FoundationOptions,
    selection: Q1SourceSelection,
    services: Box<dyn Q1CompositionServices>,
) -> Result<(Q1EntityServices, Q1SourceComposition), Q1Error> {
    let mut services = services;
    let skill = {
        let value = services.cvar("skill");
        if value >= 3.0 {
            3
        } else if value >= 2.0 {
            2
        } else if value >= 1.0 {
            1
        } else {
            0
        }
    };
    let teamplay = services.cvar("teamplay") as i32;
    let gravity = services.cvar("sv_gravity");
    let mut options = options;
    if selection.program == Q1SourceProgram::Id1 {
        options.precache_program = Some(Q1PrecacheProgram::Id1);
    }
    options.skill = skill;
    options.teamplay = Some(teamplay);
    options.gravity = gravity;
    let mut game = Q1EntityServices::new(host, options)?;
    let composition = Q1SourceComposition::register(&mut game, selection, services)?;
    Ok((game, composition))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::addons::context::attach_test_player;
    use crate::q1::base::provider::Q1CampaignState;
    use crate::q1::composition::types::{FakeCompositionServices, FakeSink};
    use crate::q1::foundation::gameplay::{DamageDelivery, DamageRequest};
    use crate::q1::foundation::types::ZERO;
    use crate::q1::missionpacks::types::{test_game, test_options};

    fn selection(program: Q1SourceProgram) -> Q1SourceSelection {
        Q1SourceSelection {
            program,
            campaign: Box::new(Q1CampaignState::new(0, 1)),
            registered: true,
            official_campaign: true,
        }
    }

    fn admission(slot: i32, name: &str) -> Q1ClientAdmission {
        Q1ClientAdmission {
            slot,
            userinfo: vec![
                (String::from("name"), String::from(name)),
                (String::from("topcolor"), String::from("2")),
                (String::from("bottomcolor"), String::from("4")),
            ],
        }
    }

    /// Register a composition on a leaked game: registries key by
    /// game address, so leaking keeps every test address unique.
    fn setup(program: Q1SourceProgram) -> (&'static mut Q1EntityServices, Q1SourceComposition, Arc<Mutex<FakeSink>>) {
        setup_with_sink(program, FakeCompositionServices::new())
    }

    fn setup_with_sink(
        program: Q1SourceProgram,
        mut services: FakeCompositionServices,
    ) -> (&'static mut Q1EntityServices, Q1SourceComposition, Arc<Mutex<FakeSink>>) {
        let game: &'static mut Q1EntityServices = Box::leak(Box::new(test_game()));
        let sink = Arc::new(Mutex::new(FakeSink::default()));
        services.sink = Some(sink.clone());
        let composition =
            Q1SourceComposition::register(game, selection(program), Box::new(services)).expect("register");
        (game, composition, sink)
    }

    /// Attach a player entity and admit it as a client.
    fn join(game: &mut Q1EntityServices, composition: &Q1SourceComposition, slot: i32, name: &str) -> OwnedActor {
        let player = attach_test_player(game);
        let owned = game.host.actors.resolve_owned(&player).expect("owned");
        composition
            .attach(game, &owned, &admission(slot, name))
            .expect("attach");
        composition.spawned(game, owned.id(), true).expect("spawned");
        owned
    }

    fn death(target: &ActorId, attacker: Option<ActorId>) -> DamageDecision {
        use qa_core::identity::ProviderId;
        use qa_core::time::SourceTime;

        DamageDecision {
            request: DamageRequest {
                attack: crate::q1::foundation::gameplay::AttackProvenance {
                    sequence: 1,
                    time: SourceTime::Seconds(0.0),
                    attacker,
                    inflictor: None,
                    originating_projectile: None,
                    weapon: None,
                    weapon_provider: ProviderId::new("q1", "test"),
                    damage_powerup_owner: None,
                    combat_provider: ProviderId::new("q1", "test"),
                    inventory_provider: ProviderId::new("q1", "test"),
                    movement_provider: ProviderId::new("q1", "test"),
                    cause: AttackCause::Q1 {
                        death_type: String::new(),
                        armor_effect: None,
                    },
                },
                target: target.clone(),
                amount: 100.0,
                knockback: 0.0,
                direction: ZERO,
                point: ZERO,
                normal: ZERO,
                delivery: DamageDelivery::Direct,
            },
            mutations: Vec::new(),
            applied_damage: 100.0,
            reaction: DamageReaction::Death,
            feedback: None,
        }
    }

    #[test]
    fn attach_spawn_input_flow() {
        let (game, composition, sink) = setup(Q1SourceProgram::Id1);
        let owned = join(&mut *game, &composition, 0, "Player");
        let client = composition.test_client(owned.id()).expect("client");
        assert_eq!(client.team, 5);
        assert_eq!(client.name(), "Player");
        composition
            .input(
                game,
                owned.id(),
                &Q1SourceInput {
                    attack: true,
                    jump: false,
                    use_action: true,
                    impulse: 9,
                },
            )
            .expect("input");
        assert!(game.player_ref(owned.id()).expect("player").attack_held);
        assert!(composition.test_client(owned.id()).expect("client").use_action);
        assert!(composition.impulse(&mut *game, owned.id()).expect("impulse"));
        assert_eq!(composition.test_client(owned.id()).expect("client").impulse, 0);
        assert_eq!(
            game.host
                .inventory
                .count(owned.id(), &String::from("q1:weapon/lightning")),
            1.0
        );
        assert!(sink
            .lock()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event, Q1CompositionEvent::Client(_))));
        composition.pre_frame(&mut *game, 0.016).expect("frame");
        assert!(!composition.impulse(&mut *game, owned.id()).expect("idle"));
    }

    #[test]
    fn userinfo_observer_and_target() {
        let (game, composition, _sink) = setup(Q1SourceProgram::Id1);
        let owned = join(&mut *game, &composition, 0, "Player");
        composition
            .userinfo(game, owned.id(), &[(String::from("name"), String::from("Renamed"))])
            .expect("userinfo");
        assert_eq!(composition.test_client(owned.id()).expect("client").name(), "Renamed");
        assert!(!composition.no_target(owned.id()));
        composition.set_no_target(owned.id(), true).expect("target");
        assert!(composition.no_target(owned.id()));
    }

    #[test]
    fn respawn_restarts_single_player() {
        let (game, composition, sink) = setup(Q1SourceProgram::Id1);
        let owned = join(&mut *game, &composition, 0, "Player");
        composition
            .request_respawn(&mut *game, owned.id(), None)
            .expect("respawn");
        assert_eq!(sink.lock().unwrap().restarts.as_slice(), [(String::new(), 0)]);
    }

    #[test]
    fn travel_round_trip() {
        let (game, composition, _sink) = setup(Q1SourceProgram::Id1);
        let owned = join(&mut *game, &composition, 0, "Player");
        let fresh = composition.new_travel(&mut *game).expect("new");
        assert_eq!(fresh.health, 100.0);
        let captured = composition.capture_travel(&mut *game, &owned).expect("capture");
        let decoded = composition.decode_travel(&mut *game, &captured).expect("decode");
        composition.admit_travel(&mut *game, &owned, &decoded).expect("admit");
        assert!(composition.drop_inventory(&mut *game, &owned).expect("drop").is_none());
    }

    #[test]
    fn intermission_exit_and_finale() {
        let (game, composition, _sink) = setup(Q1SourceProgram::Id1);
        let owned = join(&mut *game, &composition, 0, "Player");
        let result = composition
            .request_intermission_exit(&mut *game, false, None)
            .expect("exit");
        assert_eq!(result, Q1IntermissionResult::Waiting);
        composition.dismiss_finale(&mut *game).expect("dismiss");
        let pose = composition.character_pose(&mut *game, owned.id()).expect("pose");
        assert_eq!(pose, Q1CharacterSourcePose::default());
        assert!(composition.fall_damage_allowed(&*game, owned.id()));
    }

    #[test]
    fn disconnect_and_release() {
        let (game, composition, sink) = setup(Q1SourceProgram::Id1);
        let owned = join(&mut *game, &composition, 0, "Player");
        composition.disconnect(&mut *game, owned.id()).expect("disconnect");
        assert!(sink
            .lock()
            .unwrap()
            .events
            .iter()
            .any(|event| matches!(event, Q1CompositionEvent::ClientLeft { slot: 0, .. })));
        assert_eq!(sink.lock().unwrap().disconnected.as_slice(), [owned.id().clone()]);
        game.release_actor(&owned).expect("release");
        assert!(composition.test_client(owned.id()).is_none());
        let _ = composition.fired(&mut *game, owned.id(), None);
    }

    #[test]
    fn death_records_once() {
        let (game, composition, _sink) = setup(Q1SourceProgram::Id1);
        let victim = join(&mut *game, &composition, 0, "Victim");
        let attacker = join(&mut *game, &composition, 1, "Killer");
        let decision = death(victim.id(), Some(attacker.id().clone()));
        composition
            .before_reaction(&mut *game, &victim, &decision)
            .expect("death");
        assert!(composition.test_client(victim.id()).expect("client").death_recorded);
        composition
            .before_reaction(&mut *game, &victim, &decision)
            .expect("again");
    }

    #[test]
    fn foreign_absent_skips_weapons() {
        let mut services = FakeCompositionServices::new();
        services.foreign_absent = true;
        let (game, composition, _sink) = setup_with_sink(Q1SourceProgram::Id1, services);
        let owned = join(&mut *game, &composition, 0, "Player");
        composition
            .input(
                game,
                owned.id(),
                &Q1SourceInput {
                    attack: false,
                    jump: false,
                    use_action: false,
                    impulse: 7,
                },
            )
            .expect("input");
        assert!(composition.impulse(&mut *game, owned.id()).expect("impulse"));
    }

    #[test]
    fn ctf_program_registers() {
        let (game, composition, _sink) = setup(Q1SourceProgram::Ctf);
        assert!(game.state_extensions.contains_key("q1:source-clients"));
        let owned = join(&mut *game, &composition, 0, "Player");
        composition.player_pre_think(&mut *game, owned.id()).expect("pre");
        composition.player_post_think(&mut *game, owned.id()).expect("post");
        let _ = composition.character_pose(&mut *game, owned.id()).expect("pose");
        assert!(composition.fall_damage_allowed(&*game, owned.id()));
        game.create("info_player_start", None, None).expect("spot");
        composition.suicide(&mut *game, owned.id()).expect("suicide");
    }

    #[test]
    fn missionpack_program_registers() {
        let (game, composition, _sink) = setup(Q1SourceProgram::Hipnotic);
        let owned = join(&mut *game, &composition, 0, "Player");
        composition.player_post_think(&mut *game, owned.id()).expect("post");
        assert!(!composition.impulse(&mut *game, owned.id()).expect("idle"));
        let _ = composition.character_pose(&mut *game, owned.id()).expect("pose");
        let (rogue_game, rogue_comp, _) = setup(Q1SourceProgram::Rogue);
        let rogue = join(&mut *rogue_game, &rogue_comp, 0, "Rogue");
        rogue_comp
            .player_post_think(&mut *rogue_game, rogue.id())
            .expect("post");
        rogue_comp.pre_frame(&mut *rogue_game, 0.016).expect("frame");
    }

    #[test]
    fn campaign_program_registers() {
        let game: &'static mut Q1EntityServices = Box::leak(Box::new(test_game()));
        let mut services = FakeCompositionServices::new();
        services.sink = Some(Arc::new(Mutex::new(FakeSink::default())));
        let composition =
            Q1SourceComposition::register(game, selection(Q1SourceProgram::Mg3), Box::new(services)).expect("register");
        let owned = join(&mut *game, &composition, 0, "Player");
        composition.player_post_think(&mut *game, owned.id()).expect("post");
        composition.pre_frame(&mut *game, 0.016).expect("frame");
        let (mg1_game, mg1_comp, _) = setup(Q1SourceProgram::Mg1);
        let one = join(&mut *mg1_game, &mg1_comp, 0, "One");
        mg1_comp
            .request_respawn(&mut *mg1_game, one.id(), None)
            .expect("respawn");
        let (dopa_game, dopa_comp, _) = setup(Q1SourceProgram::Dopa);
        join(&mut *dopa_game, &dopa_comp, 0, "Two");
    }

    #[test]
    fn create_snapshots_options() {
        let mut options = test_options();
        options.skill = 0;
        let mut services = FakeCompositionServices::new();
        services.cvars.insert(String::from("skill"), 3.0);
        services.cvars.insert(String::from("teamplay"), 2.0);
        services.cvars.insert(String::from("sv_gravity"), 100.0);
        let (host, _) = crate::q1::foundation::host::mock::mock_host();
        let (game, composition) =
            create_q1_source_composition(host, options, selection(Q1SourceProgram::Id1), Box::new(services))
                .expect("create");
        assert_eq!(game.options().skill, 3);
        assert_eq!(game.options().teamplay, Some(2));
        assert_eq!(game.options().gravity, 100.0);
        assert_eq!(composition.program(), Q1SourceProgram::Id1);
        Box::leak(Box::new(game));
    }
}
