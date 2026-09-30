//! Q1 source composition root (`src/content/composition/q1`, barrel `index.ts`).

pub mod clients;
pub mod commands;
pub mod expansion_arsenal;
pub mod give;
pub mod runtime;
pub mod types;

pub use clients::{Q1SourceClient, Q1SourceClients};
pub use commands::{base_q1_impulse, q1_weapon_impulse};
pub use expansion_arsenal::{register_selected_q1_mission_weapons, SelectedQ1Arsenal, SelectedQ1Program};
pub use give::give_q1;
pub use runtime::{create_q1_source_composition, Q1SourceComposition};
pub use types::{
    Q1ClientAdmission, Q1ClientSnapshot, Q1CompositionCheatCategory, Q1CompositionEvent, Q1CompositionPromptChoice,
    Q1CompositionServices, Q1SelectedPlayer, Q1SourceInput, Q1SourceProgram, Q1SourceSelection,
};
