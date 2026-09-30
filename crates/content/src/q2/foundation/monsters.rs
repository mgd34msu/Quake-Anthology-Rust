//! Q2 monster runner (`src/content/q2/foundation/monsters/index.ts`).
//!
//! Animation data and callbacks extend without copying the frame or
//! perception runner. The donor's `Q2Monsters` class becomes arena state
//! ([`MonsterRuntime`]) plus free functions; contexts are transient
//! [`MonsterContext`](types::MonsterContext) facades built per dispatch.
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use std::collections::HashMap;
use std::rc::Rc;

use qa_core::identity::{ActorId, OwnedActor};

use crate::monsters::MonsterMission;

use self::types::{Q2MonsterDefinition, SourceCombatMode};

pub mod perception;
pub mod types;

/// External path follower (`Q2PathFollower`).
///
/// The application wires cross-product followers here; the game only
/// consults them for actors without a monster context.
pub trait Q2PathFollower {
    /// Follower actor.
    fn actor(&self) -> &OwnedActor;
    /// Move target.
    fn move_target(&self) -> Option<&ActorId>;
    /// Enemy.
    fn enemy(&self) -> Option<&ActorId>;
    /// Advance toward a goal.
    fn advance(&mut self, name: &str, goal: Option<&ActorId>, pause_until: f64);
}

/// External combat follower (`Q2CombatFollower`).
pub trait Q2CombatFollower {
    /// Move target.
    fn move_target(&self) -> Option<&ActorId>;
    /// Enemy.
    fn enemy(&self) -> Option<&ActorId>;
    /// Old enemy.
    fn old_enemy(&self) -> Option<&ActorId>;
    /// Activator.
    fn activator(&self) -> Option<&ActorId>;
    /// Whether walking.
    fn walking(&self) -> bool;
    /// Advance toward a target.
    fn advance(&mut self, target: &str, goal: Option<&ActorId>, move_target: Option<&ActorId>);
    /// Hold position.
    fn hold(&mut self);
    /// Finish the follow.
    fn finish(&mut self);
}

/// Pending rerelease monster damage.
#[derive(Debug, Clone)]
pub struct PendingMonsterDamage {
    /// Death reaction.
    pub reaction: crate::q2::support::contracts::DeathReaction,
    /// Attack provenance.
    pub attack: Option<crate::q2::support::contracts::AttackProvenance>,
}

/// Monster arena runtime.
pub struct MonsterRuntime {
    /// Definitions by classname.
    pub definitions: HashMap<String, Rc<Q2MonsterDefinition>>,
    /// Edition definitions by edition and classname.
    pub edition_definitions: HashMap<crate::q2::foundation::host::Q2Edition, HashMap<String, Rc<Q2MonsterDefinition>>>,
    /// Monster states by actor.
    pub states: HashMap<ActorId, self::types::MonsterState>,
    /// Resolved definitions by actor.
    pub actor_definitions: HashMap<ActorId, Rc<Q2MonsterDefinition>>,
    /// Perception state.
    pub perception: perception::PerceptionRuntime,
    /// Pending rerelease damage by actor.
    pub pending_damage: HashMap<ActorId, PendingMonsterDamage>,
    /// Whether release cleanup is connected.
    pub connected: bool,
    /// Source combat rules.
    pub source_combat: SourceCombatMode,
    /// Whether rogue hint paths are active.
    pub hint_paths: bool,
    /// Monster mission hook (composition leaves this unset; the donor's
    /// optional hook is preserved for parity).
    pub mission_hook: Option<Box<dyn FnMut(&ActorId) -> Option<Box<dyn MonsterMission>>>>,
    /// External path follower factory.
    pub external_path_follower: Option<Box<dyn FnMut(&ActorId) -> Option<Box<dyn Q2PathFollower>>>>,
    /// External combat follower factory.
    pub external_combat_follower: Option<Box<dyn FnMut(&ActorId) -> Option<Box<dyn Q2CombatFollower>>>>,
}

impl Default for MonsterRuntime {
    fn default() -> Self {
        Self {
            definitions: HashMap::new(),
            edition_definitions: HashMap::new(),
            states: HashMap::new(),
            actor_definitions: HashMap::new(),
            perception: perception::PerceptionRuntime::default(),
            pending_damage: HashMap::new(),
            connected: false,
            source_combat: SourceCombatMode::default(),
            hint_paths: false,
            mission_hook: None,
            external_path_follower: None,
            external_combat_follower: None,
        }
    }
}

impl MonsterRuntime {
    /// Require a monster state (callback invariant).
    pub fn require_state(&self, actor: &ActorId) -> &self::types::MonsterState {
        self.states
            .get(actor)
            .unwrap_or_else(|| panic!("Missing Q2 source monster context {}", actor.slot()))
    }

    /// Require a monster state, mutably (callback invariant).
    pub fn require_state_mut(&mut self, actor: &ActorId) -> &mut self::types::MonsterState {
        self.states
            .get_mut(actor)
            .unwrap_or_else(|| panic!("Missing Q2 source monster context {}", actor.slot()))
    }

    /// Require a resolved monster definition (callback invariant).
    pub fn require_definition(&self, actor: &ActorId) -> Rc<Q2MonsterDefinition> {
        self.actor_definitions
            .get(actor)
            .cloned()
            .unwrap_or_else(|| panic!("Missing Q2 source monster definition {}", actor.slot()))
    }

    /// Drop monster state after an actor release.
    pub fn on_actor_released(&mut self, actor: &ActorId) {
        self.states.remove(actor);
        self.actor_definitions.remove(actor);
        self.pending_damage.remove(actor);
        self.perception.release(actor);
    }
}
