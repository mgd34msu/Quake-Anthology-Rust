//! Q1 source composition contracts (`src/content/composition/q1/types.ts`).

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::Vec3;

use crate::contract::{ItemId, SharedGrappleControl};
use crate::q1::addons::context::Q1AddonEvent;
use crate::q1::addons::ctf::types::{CtfStatus, CtfTeam};
use crate::q1::base::provider::Q1CampaignBinding;
use crate::q1::base::rules::{Q1DeathWater, Q1SourceFinale};
use crate::q1::base::travel::Q1TravelState;
use crate::q1::foundation::entity::Q1Actor;
use crate::q1::foundation::entity_services::Q1EntityServices;

/// Selectable Q1 source program (`Q1SourceProgram`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1SourceProgram {
    /// Original id1.
    Id1,
    /// Hipnotic mission pack.
    Hipnotic,
    /// Rogue mission pack.
    Rogue,
    /// Dimension of the Past.
    Dopa,
    /// Machinegames episode 1 (horde-capable).
    Mg1,
    /// Machinegames episode 3.
    Mg3,
    /// ThreeWave capture the flag.
    Ctf,
}

impl Q1SourceProgram {
    /// Donor program id text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Q1SourceProgram::Id1 => "id1",
            Q1SourceProgram::Hipnotic => "hipnotic",
            Q1SourceProgram::Rogue => "rogue",
            Q1SourceProgram::Dopa => "dopa",
            Q1SourceProgram::Mg1 => "mg1",
            Q1SourceProgram::Mg3 => "mg3",
            Q1SourceProgram::Ctf => "ctf",
        }
    }

    /// Parse a donor program id.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "id1" => Some(Q1SourceProgram::Id1),
            "hipnotic" => Some(Q1SourceProgram::Hipnotic),
            "rogue" => Some(Q1SourceProgram::Rogue),
            "dopa" => Some(Q1SourceProgram::Dopa),
            "mg1" => Some(Q1SourceProgram::Mg1),
            "mg3" => Some(Q1SourceProgram::Mg3),
            "ctf" => Some(Q1SourceProgram::Ctf),
            _ => None,
        }
    }
}

/// Selected source program binding (`Q1SourceSelection`). The campaign
/// binding moves into the registered base content; composition reads the
/// shared flags back through the base registry.
pub struct Q1SourceSelection {
    /// Selected program.
    pub program: Q1SourceProgram,
    /// Campaign binding shared with the base content.
    pub campaign: Box<dyn Q1CampaignBinding>,
    /// Whether the game is registered.
    pub registered: bool,
    /// Whether the official campaign is selected.
    pub official_campaign: bool,
}

/// Native client admission (`Q1ClientAdmission`). The actor's source
/// edict remains slot + 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1ClientAdmission {
    /// Native client index, zero based.
    pub slot: i32,
    /// Userinfo pairs in donor order.
    pub userinfo: Vec<(String, String)>,
}

/// Per-frame source input (`Q1SourceInput`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Q1SourceInput {
    /// Attack held.
    pub attack: bool,
    /// Jump held.
    pub jump: bool,
    /// Use held.
    pub use_action: bool,
    /// Pending impulse.
    pub impulse: i32,
}

/// Selected session player observation (`Q1SelectedPlayer`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1SelectedPlayer {
    /// Source deadflag (`0..=3`).
    pub dead_flag: i32,
    /// Whether the player is a bot.
    pub is_bot: bool,
    /// View angles.
    pub view_angles: Vec3,
    /// View offset.
    pub view_offset: Vec3,
    /// Source pose frame.
    pub frame: i32,
    /// Water type.
    pub water_type: Q1DeathWater,
    /// Water depth level (`0..=3`).
    pub water_level: i32,
    /// Teleport control lock expiry in seconds.
    pub teleport_until: f64,
}

/// Published client snapshot (`Q1ClientSnapshot`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1ClientSnapshot {
    /// Client actor.
    pub actor: ActorId,
    /// Native client slot.
    pub slot: i32,
    /// Display name.
    pub name: String,
    /// Frag count.
    pub frags: f64,
    /// Shirt color.
    pub shirt: i32,
    /// Pants color.
    pub pants: i32,
    /// Team number.
    pub team: i32,
    /// Whether the client observes.
    pub observer: bool,
    /// Whether the client is untargetable.
    pub no_target: bool,
    /// Userinfo pairs in donor order.
    pub userinfo: Vec<(String, String)>,
}

/// Team prompt choice (`prompt` event choice).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q1CompositionPromptChoice {
    /// Choice label.
    pub label: String,
    /// Impulse fired by the choice.
    pub impulse: i32,
}

/// Source composition event (`Q1CompositionEvent`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q1CompositionEvent {
    /// Addon event.
    Addon(Q1AddonEvent),
    /// Client snapshot.
    Client(Q1ClientSnapshot),
    /// Client left.
    ClientLeft {
        /// Departed actor.
        actor: ActorId,
        /// Released slot.
        slot: i32,
    },
    /// CTF scoreboard status.
    CtfStatus {
        /// Notified actor.
        actor: ActorId,
        /// Scoreboard status.
        status: CtfStatus,
    },
    /// CTF capture.
    CtfCapture {
        /// Capturing team.
        team: CtfTeam,
        /// Capture total.
        total: i32,
    },
    /// Team prompt.
    Prompt {
        /// Prompted actor.
        actor: ActorId,
        /// Prompt title.
        title: String,
        /// Prompt choices.
        choices: Vec<Q1CompositionPromptChoice>,
    },
    /// Clear the team prompt.
    ClearPrompt {
        /// Cleared actor.
        actor: ActorId,
    },
    /// Source log line.
    SourceLog {
        /// Acting actor.
        actor: ActorId,
        /// Logged action.
        action: String,
    },
    /// Developer message.
    DeveloperMessage {
        /// Message text.
        text: String,
    },
    /// Level presentation.
    LevelPresentation(Q1SourceFinale),
}

/// Cheat arsenal category (`cheatArsenal` category).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1CompositionCheatCategory {
    /// Grant weapons.
    Weapons,
    /// Grant ammunition.
    Ammo,
}

impl Q1CompositionCheatCategory {
    /// Donor category text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Q1CompositionCheatCategory::Weapons => "weapons",
            Q1CompositionCheatCategory::Ammo => "ammo",
        }
    }
}

/// Operations on the already selected session movement, character,
/// arsenal and UI (`Q1CompositionServices`).
///
/// Implementations must not call back into composition entry points:
/// composition holds its state lock while invoking services, so a
/// reentrant call would deadlock. Events are pure sinks.
pub trait Q1CompositionServices: Send {
    /// Shared grapple control, if the session supplies one.
    fn shared_grapple(&self) -> Option<&dyn SharedGrappleControl> {
        None
    }

    /// Read an engine variable.
    fn cvar(&mut self, name: &str) -> f64;

    /// Write an engine variable.
    fn set_cvar(&mut self, name: &str, value: &str);

    /// Emit a composition event.
    fn emit(&mut self, event: Q1CompositionEvent);

    /// Observe the selected player.
    fn selected_player(&mut self, actor: &ActorId) -> Q1SelectedPlayer;

    /// Set observer mode on the selected session.
    fn set_observer(&mut self, actor: &ActorId, enabled: bool);

    /// Reset the selected character/movement and apply the supplied
    /// source travel once.
    fn place_player(&mut self, actor: &OwnedActor, spot: &Q1Actor, travel: &Q1TravelState);

    /// Disconnect the actor.
    fn disconnect(&mut self, actor: &ActorId);

    /// Teleport the actor.
    fn teleport(&mut self, actor: &ActorId, origin: Vec3, angles: Vec3, velocity: Vec3, until: f64);

    /// Foreign weapon game for the actor. `None` selects the main
    /// game; `Some` null pointer skips weapon logic; otherwise the
    /// pointer must be disjoint from the calling game and stay valid
    /// for the duration of the calling composition entry point.
    fn weapon_services(&mut self, actor: &ActorId) -> Option<*mut Q1EntityServices>;

    /// Grant a cheat arsenal through a foreign selected arsenal.
    /// Defaults to refusing, like the donor's optional hook.
    fn cheat_arsenal(&mut self, _actor: &ActorId, _category: Option<Q1CompositionCheatCategory>) -> bool {
        false
    }

    /// Grant through a foreign selected item path. Defaults to
    /// refusing, like the donor's optional hook.
    fn give_selected_item(&mut self, _actor: &ActorId, _args: &[String]) -> bool {
        false
    }

    /// Selected weapon item, if any.
    fn selected_weapon(&mut self, actor: &ActorId) -> Option<ItemId>;

    /// Selected ammunition item, if any.
    fn selected_ammo(&mut self, actor: &ActorId) -> Option<ItemId>;

    /// Select a weapon on the selected arsenal.
    fn select_weapon(&mut self, actor: &ActorId, item: &ItemId) -> bool;

    /// Note a weapon change.
    fn weapon_changed(&mut self, actor: &ActorId, acquired: Option<&ItemId>);

    /// Whether the team prompt is supported.
    fn prompt_supported(&mut self, actor: &ActorId) -> bool;

    /// Start a source session restart, including `SetNewParms` rather
    /// than ordinary level carry.
    fn restart_session(&mut self, map: &str, starting_server_flags: i32);

    /// Finish the campaign.
    fn finish_campaign(&mut self);
}

/// Shared sink for runtime tests, which cannot reach the boxed fake.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct FakeSink {
    /// Emitted events.
    pub events: Vec<Q1CompositionEvent>,
    /// Session restarts as `(map, flags)`.
    pub restarts: Vec<(String, i32)>,
    /// Disconnected actors.
    pub disconnected: Vec<ActorId>,
    /// Placed players as `(actor, spot)`.
    pub placed: Vec<(ActorId, ActorId)>,
}

/// Recording services shared by every composition test module.
#[cfg(test)]
pub(crate) struct FakeCompositionServices {
    /// Engine variables.
    pub cvars: std::collections::HashMap<String, f64>,
    /// Emitted events.
    pub events: Vec<Q1CompositionEvent>,
    /// Selected player observations.
    pub players: std::collections::HashMap<ActorId, Q1SelectedPlayer>,
    /// Observer flags.
    pub observers: std::collections::HashMap<ActorId, bool>,
    /// Placed players as `(actor, spot, has_travel)`.
    pub placed: Vec<(ActorId, ActorId, bool)>,
    /// Disconnected actors.
    pub disconnected: Vec<ActorId>,
    /// Teleports as `(actor, origin, angles, velocity, until)`.
    pub teleports: Vec<(ActorId, Vec3, Vec3, Vec3, f64)>,
    /// Selected weapons.
    pub weapons: std::collections::HashMap<ActorId, Option<ItemId>>,
    /// Selected ammunition.
    pub ammo: std::collections::HashMap<ActorId, Option<ItemId>>,
    /// Weapon selections.
    pub selections: Vec<(ActorId, ItemId)>,
    /// Weapon changes as `(actor, acquired)`.
    pub changes: Vec<(ActorId, Option<ItemId>)>,
    /// Actors with prompt support.
    pub prompts: std::collections::HashSet<ActorId>,
    /// Session restarts as `(map, flags)`.
    pub restarts: Vec<(String, i32)>,
    /// Campaign finishes.
    pub finishes: u32,
    /// Cheat arsenal grants.
    pub arsenals: Vec<(ActorId, Option<Q1CompositionCheatCategory>)>,
    /// Foreign item grants; true entries consume the grant.
    pub foreign_items: std::collections::HashMap<ActorId, bool>,
    /// Shared sink mirror.
    pub sink: Option<std::sync::Arc<std::sync::Mutex<FakeSink>>>,
    /// Report a foreign arsenal without a game.
    pub foreign_absent: bool,
}

#[cfg(test)]
impl FakeCompositionServices {
    /// Fresh fake with default observations.
    pub(crate) fn new() -> Self {
        Self {
            cvars: std::collections::HashMap::new(),
            events: Vec::new(),
            players: std::collections::HashMap::new(),
            observers: std::collections::HashMap::new(),
            placed: Vec::new(),
            disconnected: Vec::new(),
            teleports: Vec::new(),
            weapons: std::collections::HashMap::new(),
            ammo: std::collections::HashMap::new(),
            selections: Vec::new(),
            changes: Vec::new(),
            prompts: std::collections::HashSet::new(),
            restarts: Vec::new(),
            finishes: 0,
            arsenals: Vec::new(),
            foreign_items: std::collections::HashMap::new(),
            sink: None,
            foreign_absent: false,
        }
    }

    /// Default selected player observation.
    pub(crate) fn selected() -> Q1SelectedPlayer {
        Q1SelectedPlayer {
            dead_flag: 0,
            is_bot: false,
            view_angles: crate::q1::foundation::types::ZERO,
            view_offset: crate::q1::foundation::types::ZERO,
            frame: 0,
            water_type: Q1DeathWater::Empty,
            water_level: 0,
            teleport_until: 0.0,
        }
    }
}

#[cfg(test)]
impl Q1CompositionServices for FakeCompositionServices {
    fn cvar(&mut self, name: &str) -> f64 {
        self.cvars.get(name).copied().unwrap_or(0.0)
    }

    fn set_cvar(&mut self, name: &str, value: &str) {
        self.cvars.insert(name.to_string(), value.parse::<f64>().unwrap_or(0.0));
    }

    fn emit(&mut self, event: Q1CompositionEvent) {
        self.events.push(event.clone());
        if let Some(sink) = &self.sink {
            sink.lock().unwrap().events.push(event);
        }
    }

    fn selected_player(&mut self, actor: &ActorId) -> Q1SelectedPlayer {
        self.players.get(actor).cloned().unwrap_or_else(Self::selected)
    }

    fn set_observer(&mut self, actor: &ActorId, enabled: bool) {
        self.observers.insert(actor.clone(), enabled);
    }

    fn place_player(&mut self, actor: &OwnedActor, spot: &Q1Actor, _travel: &Q1TravelState) {
        self.placed.push((actor.id().clone(), spot.actor.id().clone(), true));
        if let Some(sink) = &self.sink {
            sink.lock()
                .unwrap()
                .placed
                .push((actor.id().clone(), spot.actor.id().clone()));
        }
    }

    fn disconnect(&mut self, actor: &ActorId) {
        self.disconnected.push(actor.clone());
        if let Some(sink) = &self.sink {
            sink.lock().unwrap().disconnected.push(actor.clone());
        }
    }

    fn teleport(&mut self, actor: &ActorId, origin: Vec3, angles: Vec3, velocity: Vec3, until: f64) {
        self.teleports.push((actor.clone(), origin, angles, velocity, until));
    }

    fn weapon_services(&mut self, _actor: &ActorId) -> Option<*mut Q1EntityServices> {
        if self.foreign_absent {
            Some(std::ptr::null_mut())
        } else {
            None
        }
    }

    fn cheat_arsenal(&mut self, actor: &ActorId, category: Option<Q1CompositionCheatCategory>) -> bool {
        self.arsenals.push((actor.clone(), category));
        false
    }

    fn give_selected_item(&mut self, actor: &ActorId, _args: &[String]) -> bool {
        self.foreign_items.get(actor).copied().unwrap_or(false)
    }

    fn selected_weapon(&mut self, actor: &ActorId) -> Option<ItemId> {
        self.weapons.get(actor).cloned().unwrap_or(None)
    }

    fn selected_ammo(&mut self, actor: &ActorId) -> Option<ItemId> {
        self.ammo.get(actor).cloned().unwrap_or(None)
    }

    fn select_weapon(&mut self, actor: &ActorId, item: &ItemId) -> bool {
        self.selections.push((actor.clone(), item.clone()));
        true
    }

    fn weapon_changed(&mut self, actor: &ActorId, acquired: Option<&ItemId>) {
        self.changes.push((actor.clone(), acquired.cloned()));
    }

    fn prompt_supported(&mut self, actor: &ActorId) -> bool {
        self.prompts.contains(actor)
    }

    fn restart_session(&mut self, map: &str, starting_server_flags: i32) {
        self.restarts.push((map.to_string(), starting_server_flags));
        if let Some(sink) = &self.sink {
            sink.lock()
                .unwrap()
                .restarts
                .push((map.to_string(), starting_server_flags));
        }
    }

    fn finish_campaign(&mut self) {
        self.finishes += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_ids_match_donor() {
        assert_eq!(Q1SourceProgram::Id1.as_str(), "id1");
        assert_eq!(Q1SourceProgram::Hipnotic.as_str(), "hipnotic");
        assert_eq!(Q1SourceProgram::Rogue.as_str(), "rogue");
        assert_eq!(Q1SourceProgram::Dopa.as_str(), "dopa");
        assert_eq!(Q1SourceProgram::Mg1.as_str(), "mg1");
        assert_eq!(Q1SourceProgram::Mg3.as_str(), "mg3");
        assert_eq!(Q1SourceProgram::Ctf.as_str(), "ctf");
        assert_eq!(Q1SourceProgram::parse("mg3"), Some(Q1SourceProgram::Mg3));
        assert_eq!(Q1SourceProgram::parse("quake2"), None);
        assert_eq!(Q1CompositionCheatCategory::Weapons.as_str(), "weapons");
        assert_eq!(Q1CompositionCheatCategory::Ammo.as_str(), "ammo");
    }
}
