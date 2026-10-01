//! Named source callbacks (`src/content/q2/foundation/callbacks.ts`).
//!
//! Q2 saves store source function names, exactly as the original game save
//! tables do. Callback slots hold `fn` pointers; the registry maps names
//! to pointers and (by pointer identity) back.

use std::collections::HashMap;
use std::hash::Hash;

use qa_core::identity::ActorId;

use super::host::{Q2Blocked, Q2Die, Q2GameServices, Q2Pain, Q2Think, Q2Touch, Q2TouchProject, Q2Use};
use crate::q2::support::contracts::WeaponTrajectoryUpdate;

/// Free an entity think callback (`freeQ2Entity`).
pub fn free_q2_entity(actor: ActorId, game: &mut Q2GameServices) {
    game.remove_actor(actor);
}

/// Free an entity die callback (`freeQ2Entity` as `Q2Die`).
pub fn free_q2_entity_die(
    actor: ActorId,
    game: &mut Q2GameServices,
    _reaction: crate::q2::support::contracts::DeathReaction,
) {
    game.remove_actor(actor);
}

/// One callback table's definitions.
#[derive(Debug, Clone, Default)]
pub struct Q2CallbackDefinitions {
    /// Touch callbacks with trajectory projections.
    pub trajectory: Vec<TrajectoryProjection>,
    /// Think callbacks by name.
    pub think: HashMap<&'static str, Q2Think>,
    /// Use callbacks by name.
    pub use_: HashMap<&'static str, Q2Use>,
    /// Touch callbacks by name.
    pub touch: HashMap<&'static str, Q2Touch>,
    /// Pain callbacks by name.
    pub pain: HashMap<&'static str, Q2Pain>,
    /// Die callbacks by name.
    pub die: HashMap<&'static str, Q2Die>,
    /// Blocked callbacks by name.
    pub blocked: HashMap<&'static str, Q2Blocked>,
}

/// Touch callback plus its trajectory projection.
#[derive(Debug, Clone, Copy)]
pub struct TrajectoryProjection {
    /// Touch callback.
    pub touch: Q2Touch,
    /// Trajectory projection.
    pub project: Q2TouchProject,
}

/// Named registry for one callback signature (`SourceCallbacks<T>`).
#[derive(Debug, Clone)]
struct NamedCallbacks<T: Copy + Eq + Hash> {
    functions: HashMap<&'static str, T>,
    names: HashMap<T, &'static str>,
}

impl<T: Copy + Eq + Hash> NamedCallbacks<T> {
    fn new() -> Self {
        Self {
            functions: HashMap::new(),
            names: HashMap::new(),
        }
    }

    fn register(&mut self, definitions: &HashMap<&'static str, T>) {
        let mut names: Vec<(&'static str, T)> = definitions.iter().map(|(name, callback)| (*name, *callback)).collect();
        names.sort_by_key(|(name, _)| *name);
        for (name, callback) in names {
            if let Some(previous) = self.functions.get(name) {
                if *previous != callback {
                    panic!("Duplicate Q2 source callback {name}");
                }
            }
            self.functions.insert(name, callback);
            self.names.entry(callback).or_insert(name);
        }
    }

    fn name(&self, callback: Option<T>) -> Option<&'static str> {
        let callback = callback?;
        Some(self.names.get(&callback).copied().unwrap_or_else(|| {
            panic!("Q2 save encountered an unnamed source callback");
        }))
    }

    fn resolve(&self, name: Option<&str>) -> Option<T> {
        let name = name?;
        Some(self.functions.get(name).copied().unwrap_or_else(|| {
            panic!("Q2 restore cannot resolve source callback {name}");
        }))
    }
}

/// Source callback registry (`Q2SourceCallbacks`).
#[derive(Debug, Clone)]
pub struct Q2SourceCallbacks {
    trajectories: HashMap<&'static str, Q2TouchProject>,
    think: NamedCallbacks<Q2Think>,
    use_: NamedCallbacks<Q2Use>,
    touch: NamedCallbacks<Q2Touch>,
    pain: NamedCallbacks<Q2Pain>,
    die: NamedCallbacks<Q2Die>,
    blocked: NamedCallbacks<Q2Blocked>,
}

impl Q2SourceCallbacks {
    /// Empty registry.
    pub fn new() -> Self {
        Self {
            trajectories: HashMap::new(),
            think: NamedCallbacks::new(),
            use_: NamedCallbacks::new(),
            touch: NamedCallbacks::new(),
            pain: NamedCallbacks::new(),
            die: NamedCallbacks::new(),
            blocked: NamedCallbacks::new(),
        }
    }

    /// Project a trajectory through the entity's named touch callback.
    pub fn project_trajectory(&self, actor: ActorId, game: &mut Q2GameServices, update: &WeaponTrajectoryUpdate) {
        let touch = game.entity(&actor).and_then(|entity| entity.touch);
        if let Some(name) = self.touch.name(touch) {
            if let Some(project) = self.trajectories.get(name) {
                project(actor, game, update);
            }
        }
    }

    /// Think callback name for saves.
    pub fn think_name(&self, callback: Option<Q2Think>) -> Option<&'static str> {
        self.think.name(callback)
    }

    /// Use callback name for saves.
    pub fn use_name(&self, callback: Option<Q2Use>) -> Option<&'static str> {
        self.use_.name(callback)
    }

    /// Touch callback name for saves.
    pub fn touch_name(&self, callback: Option<Q2Touch>) -> Option<&'static str> {
        self.touch.name(callback)
    }

    /// Pain callback name for saves.
    pub fn pain_name(&self, callback: Option<Q2Pain>) -> Option<&'static str> {
        self.pain.name(callback)
    }

    /// Die callback name for saves.
    pub fn die_name(&self, callback: Option<Q2Die>) -> Option<&'static str> {
        self.die.name(callback)
    }

    /// Blocked callback name for saves.
    pub fn blocked_name(&self, callback: Option<Q2Blocked>) -> Option<&'static str> {
        self.blocked.name(callback)
    }

    /// Resolve a think callback from saves.
    pub fn resolve_think(&self, name: Option<&str>) -> Option<Q2Think> {
        self.think.resolve(name)
    }

    /// Resolve a use callback from saves.
    pub fn resolve_use(&self, name: Option<&str>) -> Option<Q2Use> {
        self.use_.resolve(name)
    }

    /// Resolve a touch callback from saves.
    pub fn resolve_touch(&self, name: Option<&str>) -> Option<Q2Touch> {
        self.touch.resolve(name)
    }

    /// Resolve a pain callback from saves.
    pub fn resolve_pain(&self, name: Option<&str>) -> Option<Q2Pain> {
        self.pain.resolve(name)
    }

    /// Resolve a die callback from saves.
    pub fn resolve_die(&self, name: Option<&str>) -> Option<Q2Die> {
        self.die.resolve(name)
    }

    /// Resolve a blocked callback from saves.
    pub fn resolve_blocked(&self, name: Option<&str>) -> Option<Q2Blocked> {
        self.blocked.resolve(name)
    }

    /// Register one module's callback definitions.
    pub fn register(&mut self, definitions: &Q2CallbackDefinitions) {
        self.think.register(&definitions.think);
        self.use_.register(&definitions.use_);
        self.touch.register(&definitions.touch);
        self.pain.register(&definitions.pain);
        self.die.register(&definitions.die);
        self.blocked.register(&definitions.blocked);
        for projection in &definitions.trajectory {
            let name = self.touch.name(Some(projection.touch)).unwrap_or_else(|| {
                panic!("Trajectory projection requires a named source touch callback");
            });
            if let Some(previous) = self.trajectories.get(name) {
                if *previous as usize != projection.project as usize {
                    panic!("Duplicate Q2 trajectory projection {name}");
                }
            }
            self.trajectories.insert(name, projection.project);
        }
    }
}

impl Default for Q2SourceCallbacks {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge callback records (`mergeCallbacks` in monster registration).
pub fn merge_callbacks<T: Copy>(sources: &[HashMap<&'static str, T>]) -> HashMap<&'static str, T> {
    let mut merged = HashMap::new();
    for source in sources {
        for (name, callback) in source {
            merged.insert(*name, *callback);
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn think_a(actor: ActorId, game: &mut Q2GameServices) {
        let _ = (actor, game);
    }

    fn think_b(actor: ActorId, game: &mut Q2GameServices) {
        let _ = (actor, game);
    }

    #[test]
    fn names_and_resolves_think_callbacks() {
        let mut registry = Q2SourceCallbacks::new();
        let mut definitions = Q2CallbackDefinitions::default();
        definitions.think.insert("think_a", think_a as Q2Think);
        registry.register(&definitions);
        assert_eq!(registry.think_name(Some(think_a as Q2Think)), Some("think_a"));
        assert_eq!(registry.think_name(None), None);
        assert!(registry.resolve_think(Some("think_a")).is_some());
        assert!(registry.resolve_think(None).is_none());
        let _ = think_b;
    }

    #[test]
    #[should_panic(expected = "unnamed source callback")]
    fn unnamed_callback_panics_on_save() {
        let registry = Q2SourceCallbacks::new();
        let _ = registry.think_name(Some(think_b as Q2Think));
    }

    #[test]
    #[should_panic(expected = "cannot resolve source callback")]
    fn unknown_callback_panics_on_restore() {
        let registry = Q2SourceCallbacks::new();
        let _ = registry.resolve_think(Some("missing"));
    }
}
