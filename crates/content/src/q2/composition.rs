//! Q2 product composition (`src/content/composition/q2`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use crate::q2::foundation::host::Q2GameServices;

pub mod match_;
pub mod match_selection;
pub mod types;

pub use self::match_::Q2ProductMatch;
pub use self::match_selection::source_q2_match_selection;
pub use self::types::{
    set_q2_info_value, Q2ClassicProgram, Q2CompositionCommon, Q2CompositionEntityHooks, Q2CompositionEvent,
    Q2CompositionOptions, Q2CompositionServices, Q2CvarSource, Q2DeathmatchFlagsHooks, Q2ForeignPowerups,
    Q2MatchSelection, Q2RereleaseProgram,
};

/// Arena runtime state for product composition.
pub struct CompositionRuntime {
    /// Session services.
    pub services: Option<Q2CompositionServices>,
    /// Active rule flags.
    pub active_rules: i32,
}

impl Default for CompositionRuntime {
    fn default() -> Self {
        Self {
            services: None,
            active_rules: 0,
        }
    }
}

impl std::fmt::Debug for CompositionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositionRuntime")
            .field("services", &self.services)
            .field("active_rules", &self.active_rules)
            .finish()
    }
}

/// Session composition services.
pub fn composition_services(game: &Q2GameServices) -> &Q2CompositionServices {
    game.composition
        .services
        .as_ref()
        .expect("Q2 composition is not registered")
}

/// Emit a composition event to the session.
pub fn composition_emit(game: &Q2GameServices, event: Q2CompositionEvent) {
    (composition_services(game).emit)(event);
}
