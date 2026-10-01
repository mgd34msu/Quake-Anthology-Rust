//! Common-lived collision map controls (`cm_noAreas`, `cm_noCurves`,
//! `cm_playerCurveClip`).
//!
//! Donor provenance: `/home/buzzkill/Projects/quake-typescript/src/world/collision/q3/settings.ts`.

use std::cell::RefCell;
use std::rc::Rc;

use qa_core::cvar::{flags, CvarRegistry};

use crate::error::WorldError;

/// Collision map cvar definitions: name, default value, flags.
pub const COLLISION_MAP_CVAR_DEFINITIONS: [(&str, &str, u32); 3] = [
    ("cm_noAreas", "0", flags::CHEAT),
    ("cm_noCurves", "0", flags::CHEAT),
    ("cm_playerCurveClip", "1", flags::ARCHIVE | flags::CHEAT),
];

/// `CM_LoadMap` registers these common-lived controls before reading or
/// reusing a map. Reads stay live: traces consult the registry every call.
#[derive(Clone)]
pub struct CollisionMapSettings {
    cvars: Rc<RefCell<CvarRegistry>>,
}

impl std::fmt::Debug for CollisionMapSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CollisionMapSettings").finish_non_exhaustive()
    }
}

impl CollisionMapSettings {
    /// Borrow the shared registry.
    #[must_use]
    pub fn new(cvars: Rc<RefCell<CvarRegistry>>) -> Self {
        Self { cvars }
    }

    /// Register all collision map controls.
    pub fn register_map(&self) -> Result<(), WorldError> {
        let mut cvars = self.cvars.borrow_mut();
        for (name, value, flag) in COLLISION_MAP_CVAR_DEFINITIONS {
            cvars
                .register(name, value, flag)
                .map_err(|error| WorldError::BadCollisionRecord(error.to_string()))?;
        }
        Ok(())
    }

    /// `cm_noAreas`: every area connects to every other area.
    pub fn no_areas(&self) -> Result<bool, WorldError> {
        self.enabled("cm_noAreas")
    }

    /// `cm_noCurves`: traces skip patch surfaces.
    pub fn no_curves(&self) -> Result<bool, WorldError> {
        self.enabled("cm_noCurves")
    }

    /// `cm_playerCurveClip`: point traces clip against patches.
    pub fn player_curve_clip(&self) -> Result<bool, WorldError> {
        self.enabled("cm_playerCurveClip")
    }

    fn enabled(&self, name: &str) -> Result<bool, WorldError> {
        let cvars = self.cvars.borrow();
        let value = cvars
            .get(name)
            .ok_or_else(|| WorldError::BadCollisionRecord(format!("Collision map cvar {name} is not registered")))?;
        Ok(value.integer_value != 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    fn settings() -> CollisionMapSettings {
        CollisionMapSettings::new(Rc::new(RefCell::new(CvarRegistry::new(Dialect::Q3))))
    }

    #[test]
    fn register_map_sets_source_defaults() {
        let settings = settings();
        settings.register_map().expect("register");
        assert!(!settings.no_areas().expect("noAreas"));
        assert!(!settings.no_curves().expect("noCurves"));
        assert!(settings.player_curve_clip().expect("playerCurveClip"));
    }

    #[test]
    fn reads_are_live_and_require_registration() {
        let settings = settings();
        let error = settings.no_areas().expect_err("unregistered cvar must fail");
        assert_eq!(error.to_string(), "Collision map cvar cm_noAreas is not registered");
        settings.register_map().expect("register");
        settings
            .cvars
            .borrow_mut()
            .set("cm_noAreas", "1", true)
            .expect("toggle");
        assert!(settings.no_areas().expect("live read"));
    }
}
