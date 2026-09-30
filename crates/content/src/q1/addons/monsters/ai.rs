//! Q1 mg3 monster AI root (`src/content/q1/addons/monsters/ai`, barrel `index.ts`).

pub mod controller;
pub mod path;
pub mod targets;

pub use controller::{
    clone_monster_controller, mg3_monster_source, register_mg3_monster_callbacks, register_mg3_monster_source,
    Mg3ActionHandler, Mg3AiHandler, Mg3DieHandler, Mg3FindTargetHandler, Mg3FoundHandler, Mg3MeleeHandler, Mg3Monster,
    Mg3PainHandler, Mg3PlayHandler, Mg3RunHandler, Mg3SightHandler, Mg3SourceHooks, Mg3SourceRegistration,
    Mg3StartHandler, Mg3TryAttackHandler, Mg3UseHandler,
};
pub use path::Mg3MonsterNavigationHost;
pub use path::{register_mg3_monster_navigation, walk_mg3_path_to_goal, Mg3PathResult};
