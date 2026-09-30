//! Q1 CTF shared types (src/content/q1/addons/ctf/types.ts).

use qa_core::identity::ActorId;
use qa_core::math::{Bounds, Vec3};

use crate::contract::ItemId;

/// CTF team (`CtfTeam`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CtfTeam {
    /// Red team (`item_flag_team1`).
    Red,
    /// Blue team (`item_flag_team2`).
    Blue,
}

impl CtfTeam {
    /// Donor team text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CtfTeam::Red => "red",
            CtfTeam::Blue => "blue",
        }
    }

    /// Parse donor team text.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "red" => Some(CtfTeam::Red),
            "blue" => Some(CtfTeam::Blue),
            _ => None,
        }
    }
}

/// CTF rune (`CtfRune`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CtfRune {
    /// Resistance rune.
    Resistance,
    /// Strength rune.
    Strength,
    /// Haste rune.
    Haste,
    /// Regeneration rune.
    Regeneration,
}

impl CtfRune {
    /// Donor rune text.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            CtfRune::Resistance => "resistance",
            CtfRune::Strength => "strength",
            CtfRune::Haste => "haste",
            CtfRune::Regeneration => "regeneration",
        }
    }

    /// Rune ordinal in [`CTF_RUNES`] order.
    #[must_use]
    pub fn ordinal(self) -> usize {
        match self {
            CtfRune::Resistance => 0,
            CtfRune::Strength => 1,
            CtfRune::Haste => 2,
            CtfRune::Regeneration => 3,
        }
    }

    /// Parse donor rune text.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "resistance" => Some(CtfRune::Resistance),
            "strength" => Some(CtfRune::Strength),
            "haste" => Some(CtfRune::Haste),
            "regeneration" => Some(CtfRune::Regeneration),
            _ => None,
        }
    }
}

/// Rune roster in donor order (`CTF_RUNES`).
pub const CTF_RUNES: [CtfRune; 4] = [
    CtfRune::Resistance,
    CtfRune::Strength,
    CtfRune::Haste,
    CtfRune::Regeneration,
];

/// Teamplay flag bits (`CTF_FLAGS`).
pub struct CtfFlags;

impl CtfFlags {
    /// Protect teammates from health damage.
    pub const HEALTH_PROTECT: i64 = 1;
    /// Protect teammates from armor damage.
    pub const ARMOR_PROTECT: i64 = 2;
    /// Reflect teammate damage back to the attacker.
    pub const REFLECT_DAMAGE: i64 = 4;
    /// Penalize attacking teammates.
    pub const FRAG_PENALTY: i64 = 8;
    /// Kill attackers who frag teammates.
    pub const DEATH_PENALTY: i64 = 16;
    /// Lock team membership.
    pub const STATIC_TEAMS: i64 = 64;
    /// Allow impulse 20/21 item drops.
    pub const DROP_ITEMS: i64 = 128;
    /// Offer the team prompt on admission.
    pub const SELECT_TEAM: i64 = 1024;
    /// Disable the grapple.
    pub const DISABLE_GRAPPLE: i64 = 2048;
}

/// Flag pickup bounds (`CTF_FLAG_BOUNDS`).
pub const CTF_FLAG_BOUNDS: Bounds = Bounds {
    min: Vec3 {
        x: -16.0,
        y: -16.0,
        z: 0.0,
    },
    max: Vec3 {
        x: 16.0,
        y: 16.0,
        z: 74.0,
    },
};

/// Session input observation (`CtfInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CtfInput {
    /// Attack held.
    pub attack: bool,
    /// Jump held.
    pub jump: bool,
    /// Pending impulse.
    pub impulse: i32,
    /// Grapple weapon selected.
    pub grapple_selected: bool,
    /// View angles.
    pub view_angles: Vec3,
    /// Teleport control lock expiry in seconds.
    pub teleport_until: f64,
    /// Source pose frame of the selected character.
    pub frame: i32,
}

/// Scoreboard status (`CtfStatus`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CtfStatus {
    /// Red captures.
    pub red: i32,
    /// Blue captures.
    pub blue: i32,
    /// Packed flag state bits.
    pub flags: i32,
    /// Rune indicator bits.
    pub rune_items: i32,
}

/// Team prompt choice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CtfPromptChoice {
    /// Choice label.
    pub label: String,
    /// Impulse fired by the choice.
    pub impulse: i32,
}

/// Session services (`Q1CtfServices`). The selected session, character
/// and arsenal retain admission, score and command ownership.
pub trait Q1CtfServices: Send {
    /// Display name.
    fn name(&mut self, actor: &ActorId) -> String;
    /// Whether the actor is a bot.
    fn is_bot(&mut self, actor: &ActorId) -> bool;
    /// Current score.
    fn score(&mut self, actor: &ActorId) -> f64;
    /// Adjust the score.
    fn add_score(&mut self, actor: &ActorId, delta: f64);
    /// Team captures.
    fn captures(&mut self, team: CtfTeam) -> i32;
    /// Record a team capture.
    fn add_capture(&mut self, team: CtfTeam);
    /// Current input observation.
    fn input(&mut self, actor: &ActorId) -> CtfInput;
    /// Consume the pending impulse.
    fn consume_impulse(&mut self, actor: &ActorId);
    /// Whether the actor observes.
    fn observer(&mut self, actor: &ActorId) -> bool;
    /// Set observer mode.
    fn set_observer(&mut self, actor: &ActorId, observer: bool);
    /// Respawn at a spot.
    fn respawn(&mut self, actor: &ActorId, spot: Option<&ActorId>);
    /// Disconnect the actor.
    fn disconnect(&mut self, actor: &ActorId);
    /// Set shirt and pants colors.
    fn colors(&mut self, actor: &ActorId, shirt: i32, pants: i32);
    /// Whether the team prompt is supported.
    fn prompt_supported(&mut self, actor: &ActorId) -> bool;
    /// Show the team prompt.
    fn prompt(&mut self, actor: &ActorId, title: &str, choices: &[CtfPromptChoice]);
    /// Clear the team prompt.
    fn clear_prompt(&mut self, actor: &ActorId);
    /// Teleport the actor.
    fn teleport(&mut self, actor: &ActorId, origin: Vec3, angles: Vec3, velocity: Vec3, until: f64);
    /// Select the grapple weapon.
    fn select_grapple(&mut self, actor: &ActorId);
    /// Selected weapon item, if any.
    fn selected_weapon(&mut self, actor: &ActorId) -> Option<ItemId>;
    /// Selected ammunition item, if any.
    fn selected_ammo(&mut self, actor: &ActorId) -> Option<ItemId>;
    /// Note a weapon change.
    fn weapon_changed(&mut self, actor: &ActorId, acquired: Option<&ItemId>);
    /// Apply source-specific haste intervals and doubled nail velocity.
    fn haste(&mut self, actor: &ActorId, enabled: bool);
    /// Push scoreboard status.
    fn status(&mut self, actor: &ActorId, status: CtfStatus);
    /// Log a CTF action.
    fn log(&mut self, actor: &ActorId, action: &str);
}

/// Neutral input for actors without a test observation.
#[cfg(test)]
pub(crate) fn neutral_ctf_input() -> CtfInput {
    CtfInput {
        attack: false,
        jump: false,
        impulse: 0,
        grapple_selected: false,
        view_angles: crate::q1::foundation::types::ZERO,
        teleport_until: 0.0,
        frame: 0,
    }
}

/// Recording test services shared by every CTF test module. Tests hold
/// the state handle and assert on it after driving content.
#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct FakeCtfState {
    /// Display names.
    pub names: std::collections::HashMap<ActorId, String>,
    /// Bot actors.
    pub bots: std::collections::HashSet<ActorId>,
    /// Scores.
    pub scores: std::collections::HashMap<ActorId, f64>,
    /// Captures as `[red, blue]`.
    pub captures: [i32; 2],
    /// Input observations.
    pub inputs: std::collections::HashMap<ActorId, CtfInput>,
    /// Observer actors.
    pub observers: std::collections::HashSet<ActorId>,
    /// Respawn calls as `(actor, spot)`.
    pub respawns: Vec<(ActorId, Option<ActorId>)>,
    /// Disconnect calls.
    pub disconnects: Vec<ActorId>,
    /// Color calls as `(shirt, pants)`.
    pub colors: std::collections::HashMap<ActorId, (i32, i32)>,
    /// Prompt calls as `(actor, title, choices)`.
    pub prompts: Vec<(ActorId, String, Vec<CtfPromptChoice>)>,
    /// Actors with prompt support.
    pub prompt_supported: std::collections::HashSet<ActorId>,
    /// Teleport calls.
    pub teleports: Vec<(ActorId, Vec3, Vec3, Vec3, f64)>,
    /// Grapple selections.
    pub grapple_selected: std::collections::HashSet<ActorId>,
    /// Selected weapons.
    pub selected_weapons: std::collections::HashMap<ActorId, Option<ItemId>>,
    /// Selected ammunition.
    pub selected_ammo: std::collections::HashMap<ActorId, Option<ItemId>>,
    /// Weapon change calls.
    pub weapon_changes: Vec<(ActorId, Option<ItemId>)>,
    /// Haste flags.
    pub haste: std::collections::HashMap<ActorId, bool>,
    /// Pushed statuses.
    pub statuses: std::collections::HashMap<ActorId, CtfStatus>,
    /// Log calls.
    pub logs: Vec<(ActorId, String)>,
    /// Impulse consumptions.
    pub consumed: std::collections::HashSet<ActorId>,
}

/// Test [`Q1CtfServices`] recording into shared state.
#[cfg(test)]
pub(crate) struct FakeCtfServices {
    /// Shared state also held by the test.
    pub state: std::sync::Arc<std::sync::Mutex<FakeCtfState>>,
}

#[cfg(test)]
impl FakeCtfServices {
    /// Fresh services plus the test-owned state handle.
    pub(crate) fn new() -> (Self, std::sync::Arc<std::sync::Mutex<FakeCtfState>>) {
        let state = std::sync::Arc::new(std::sync::Mutex::new(FakeCtfState::default()));
        (Self { state: state.clone() }, state)
    }
}

#[cfg(test)]
impl Q1CtfServices for FakeCtfServices {
    fn name(&mut self, actor: &ActorId) -> String {
        self.state
            .lock()
            .expect("fake ctf")
            .names
            .get(actor)
            .cloned()
            .unwrap_or_else(|| String::from("player"))
    }

    fn is_bot(&mut self, actor: &ActorId) -> bool {
        self.state.lock().expect("fake ctf").bots.contains(actor)
    }

    fn score(&mut self, actor: &ActorId) -> f64 {
        self.state
            .lock()
            .expect("fake ctf")
            .scores
            .get(actor)
            .copied()
            .unwrap_or(0.0)
    }

    fn add_score(&mut self, actor: &ActorId, delta: f64) {
        let mut state = self.state.lock().expect("fake ctf");
        *state.scores.entry(actor.clone()).or_insert(0.0) += delta;
    }

    fn captures(&mut self, team: CtfTeam) -> i32 {
        self.state.lock().expect("fake ctf").captures[match team {
            CtfTeam::Red => 0,
            CtfTeam::Blue => 1,
        }]
    }

    fn add_capture(&mut self, team: CtfTeam) {
        self.state.lock().expect("fake ctf").captures[match team {
            CtfTeam::Red => 0,
            CtfTeam::Blue => 1,
        }] += 1;
    }

    fn input(&mut self, actor: &ActorId) -> CtfInput {
        self.state
            .lock()
            .expect("fake ctf")
            .inputs
            .get(actor)
            .copied()
            .unwrap_or_else(neutral_ctf_input)
    }

    fn consume_impulse(&mut self, actor: &ActorId) {
        let mut state = self.state.lock().expect("fake ctf");
        state.consumed.insert(actor.clone());
        if let Some(input) = state.inputs.get_mut(actor) {
            input.impulse = 0;
        }
    }

    fn observer(&mut self, actor: &ActorId) -> bool {
        self.state.lock().expect("fake ctf").observers.contains(actor)
    }

    fn set_observer(&mut self, actor: &ActorId, observer: bool) {
        let mut state = self.state.lock().expect("fake ctf");
        if observer {
            state.observers.insert(actor.clone());
        } else {
            state.observers.remove(actor);
        }
    }

    fn respawn(&mut self, actor: &ActorId, spot: Option<&ActorId>) {
        self.state
            .lock()
            .expect("fake ctf")
            .respawns
            .push((actor.clone(), spot.cloned()));
    }

    fn disconnect(&mut self, actor: &ActorId) {
        self.state.lock().expect("fake ctf").disconnects.push(actor.clone());
    }

    fn colors(&mut self, actor: &ActorId, shirt: i32, pants: i32) {
        self.state
            .lock()
            .expect("fake ctf")
            .colors
            .insert(actor.clone(), (shirt, pants));
    }

    fn prompt_supported(&mut self, actor: &ActorId) -> bool {
        self.state.lock().expect("fake ctf").prompt_supported.contains(actor)
    }

    fn prompt(&mut self, actor: &ActorId, title: &str, choices: &[CtfPromptChoice]) {
        self.state
            .lock()
            .expect("fake ctf")
            .prompts
            .push((actor.clone(), title.to_string(), choices.to_vec()));
    }

    fn clear_prompt(&mut self, _actor: &ActorId) {}

    fn teleport(&mut self, actor: &ActorId, origin: Vec3, angles: Vec3, velocity: Vec3, until: f64) {
        self.state
            .lock()
            .expect("fake ctf")
            .teleports
            .push((actor.clone(), origin, angles, velocity, until));
    }

    fn select_grapple(&mut self, actor: &ActorId) {
        self.state
            .lock()
            .expect("fake ctf")
            .grapple_selected
            .insert(actor.clone());
    }

    fn selected_weapon(&mut self, actor: &ActorId) -> Option<ItemId> {
        self.state
            .lock()
            .expect("fake ctf")
            .selected_weapons
            .get(actor)
            .cloned()
            .unwrap_or(None)
    }

    fn selected_ammo(&mut self, actor: &ActorId) -> Option<ItemId> {
        self.state
            .lock()
            .expect("fake ctf")
            .selected_ammo
            .get(actor)
            .cloned()
            .unwrap_or(None)
    }

    fn weapon_changed(&mut self, actor: &ActorId, acquired: Option<&ItemId>) {
        self.state
            .lock()
            .expect("fake ctf")
            .weapon_changes
            .push((actor.clone(), acquired.cloned()));
    }

    fn haste(&mut self, actor: &ActorId, enabled: bool) {
        self.state
            .lock()
            .expect("fake ctf")
            .haste
            .insert(actor.clone(), enabled);
    }

    fn status(&mut self, actor: &ActorId, status: CtfStatus) {
        self.state
            .lock()
            .expect("fake ctf")
            .statuses
            .insert(actor.clone(), status);
    }

    fn log(&mut self, actor: &ActorId, action: &str) {
        self.state
            .lock()
            .expect("fake ctf")
            .logs
            .push((actor.clone(), action.to_string()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_and_rune_text_round_trip() {
        assert_eq!(CtfTeam::Red.as_str(), "red");
        assert_eq!(CtfTeam::parse("blue"), Some(CtfTeam::Blue));
        assert_eq!(CtfTeam::parse("green"), None);
        assert_eq!(CtfRune::Haste.as_str(), "haste");
        assert_eq!(CtfRune::Regeneration.ordinal(), 3);
        assert_eq!(CtfRune::parse("strength"), Some(CtfRune::Strength));
        assert_eq!(
            CTF_RUNES.map(CtfRune::as_str),
            ["resistance", "strength", "haste", "regeneration"]
        );
    }

    #[test]
    fn flag_bits_and_bounds_match_donor() {
        assert_eq!(
            (
                CtfFlags::HEALTH_PROTECT,
                CtfFlags::ARMOR_PROTECT,
                CtfFlags::REFLECT_DAMAGE,
                CtfFlags::FRAG_PENALTY,
                CtfFlags::DEATH_PENALTY,
                CtfFlags::STATIC_TEAMS,
                CtfFlags::DROP_ITEMS,
                CtfFlags::SELECT_TEAM,
                CtfFlags::DISABLE_GRAPPLE,
            ),
            (1, 2, 4, 8, 16, 64, 128, 1024, 2048)
        );
        assert_eq!(CTF_FLAG_BOUNDS.min.z, 0.0);
        assert_eq!(CTF_FLAG_BOUNDS.max.z, 74.0);
    }
}
