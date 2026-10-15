//! Curve controls use the shared cvar table and its cached native defaults.
use crate::cvars::Cvars;
use qa_core::primitives::{CvarHandle, PatchClip, RuleSetId};

#[derive(Clone, Copy, Debug)]
pub struct CollisionHandles {
    no_curves: CvarHandle,
    point_curves: CvarHandle,
}
impl CollisionHandles {
    pub fn bind(vars: &Cvars) -> Result<Self, &'static str> {
        let bind = |name| vars.find(name).ok_or(name);
        Ok(Self {
            no_curves: bind("cm_noCurves")?,
            point_curves: bind("cm_playerCurveClip")?,
        })
    }
    pub fn resolve(self, vars: &Cvars) -> PatchClip {
        // These stored controls have one native default column. Their value
        // applies to every caller, independently of movement and map format.
        PatchClip::from_flags(
            vars.integer_in(self.no_curves, RuleSetId::Quake3) == 0,
            vars.integer_in(self.point_curves, RuleSetId::Quake3) != 0,
        )
    }
}
