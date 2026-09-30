//! Mission-pack monster definition types
//! (`src/content/q1/missionpacks/monsters/types.ts`).

use std::rc::Rc;

use qa_core::identity::ActorId;

use crate::q1::base::animation::{MonsterAi, MonsterFrame};
use crate::q1::base::species::MonsterSpecies;
use crate::q1::foundation::callbacks::Q1CallbackHandlers;
use crate::q1::foundation::entity_services::Q1EntityServices;

use super::runtime::{MissionMonster, Q1MissionPackMonsters};

/// Optional runtime hooks (`MissionMonsterHooks`). The donors never read
/// these; the top-level runtime assigns the horn charmer here.
#[derive(Clone, Default)]
pub struct MissionMonsterHooks {
    /// Resolve the current charmer, if any.
    pub charmer: Option<fn(&Q1EntityServices) -> Option<ActorId>>,
}

/// Frame/attack callback (`MissionAction`). Donor actions close over the
/// runtime; here they receive the monster view, which borrows it.
pub type MissionAction = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>)>;

/// Pain callback (`PackMonsterDefinition["pain"]`).
pub type MissionPain = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>, Option<&ActorId>, f64)>;
/// Death callback (`PackMonsterDefinition["die"]`).
pub type MissionDie = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>, Option<&ActorId>)>;
/// Attack-check callback (`PackMonsterDefinition["checkAttack"]`).
pub type MissionCheckAttack = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>) -> bool>;
/// Target-found callback (`PackMonsterDefinition["found"]`).
pub type MissionFound = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>, &ActorId)>;
/// Steering callback (`PackMonsterDefinition["ai"]`).
pub type MissionAi = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>, MonsterAi, f64)>;
/// Use callback (`PackMonsterDefinition["use"]`).
pub type MissionUse = Rc<dyn for<'m> Fn(&mut MissionMonster<'m>, Option<&ActorId>)>;

/// One mission-pack monster definition (`PackMonsterDefinition`).
pub struct PackMonsterDefinition {
    /// Spawn defaults. Leaked `'static` at definition build time so base
    /// views can borrow it.
    pub spec: &'static MonsterSpecies,
    /// Delegate pain/die/melee to the base game (`baseBehavior`).
    pub base_behavior: bool,
    /// Frame table slice (`super::tables::<name>::FRAMES`).
    pub frames: &'static [(&'static str, MonsterFrame)],
    /// Named frame actions.
    pub actions: Vec<(&'static str, MissionAction)>,
    /// Named source callbacks, registered under `{pack}:{name}`.
    pub callbacks: Vec<(&'static str, Q1CallbackHandlers)>,
    /// Spawn override (`spawn`).
    pub spawn: Option<MissionAction>,
    /// Start override (`start`).
    pub start: Option<MissionAction>,
    /// Pain handler (`pain`).
    pub pain: MissionPain,
    /// Death handler (`die`).
    pub die: MissionDie,
    /// Melee attack (`melee`).
    pub melee: Option<MissionAction>,
    /// Attack check (`checkAttack`).
    pub check_attack: Option<MissionCheckAttack>,
    /// Target-found handler (`found`).
    pub found: Option<MissionFound>,
    /// Steering override (`ai`).
    pub ai: Option<MissionAi>,
    /// Use handler (`use`).
    pub use_: Option<MissionUse>,
}

/// Definition factory (`PackMonsterFactory`).
pub type PackMonsterFactory =
    Rc<dyn Fn(&Q1MissionPackMonsters, &Q1EntityServices) -> Vec<PackMonsterDefinition>>;

/// Spawn function (`MissionSpawn`).
pub type MissionSpawn = Rc<dyn Fn(&mut Q1EntityServices, &ActorId)>;

/// Leak a dynamically built action name (`ai_back(3)` and friends) as a
/// `'static` key. Definitions are built once per registration, so the
/// handful of leaked names is bounded and intentional.
#[must_use]
pub fn leaked_name(text: String) -> &'static str {
    Box::leak(text.into_boxed_str())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q1::base::species::MonsterMovement;
    use crate::q1::foundation::entity::Q1MonsterSpecies;
    use crate::q1::foundation::types::ZERO;

    fn test_spec() -> &'static MonsterSpecies {
        Box::leak(Box::new(MonsterSpecies {
            species: Q1MonsterSpecies::Gremlin,
            kill_string: None,
            classnames: &["monster_gremlin"],
            model: "grem",
            head: Some("h_grem"),
            health: 100.0,
            gib_health: -35.0,
            gibs: &["gib1"],
            bounds: crate::q1::foundation::types::Bounds {
                min: ZERO,
                max: ZERO,
            },
            stand: "gremlin_stand1",
            walk: "gremlin_walk1",
            run: "gremlin_run1",
            sight: "grem/sight1.wav",
            missile: Some("Gremlin_MissileAttack"),
            melee: true,
            movement: MonsterMovement::Walk,
        }))
    }

    #[test]
    fn definition_holds_actions_and_callbacks() {
        let definition = PackMonsterDefinition {
            spec: test_spec(),
            base_behavior: false,
            frames: &[],
            actions: vec![("ping", Rc::new(|_| {}))],
            callbacks: vec![("Ping", Q1CallbackHandlers::default())],
            spawn: None,
            start: None,
            pain: Rc::new(|_, _, _| {}),
            die: Rc::new(|_, _| {}),
            melee: None,
            check_attack: Some(Rc::new(|_| true)),
            found: None,
            ai: None,
            use_: None,
        };
        assert!(!definition.base_behavior);
        assert_eq!(definition.actions.len(), 1);
        assert_eq!(definition.callbacks.len(), 1);
        assert!(definition.spawn.is_none());
        let hooks = MissionMonsterHooks::default();
        assert!(hooks.charmer.is_none());
    }
}
