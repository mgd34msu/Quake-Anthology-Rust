pub mod q1;
pub mod q2;
pub mod q3;

use qa_core::primitives::{LinkOrder, RuleSetId};
use std::num::NonZeroU32;

/// Native server cadence data; the host owns the timeline and resolves cvars.
/// Q3's below-one fallback is applied at that external settings boundary.
pub fn tick_millis(rules: RuleSetId, sv_fps: NonZeroU32) -> Option<NonZeroU32> {
    match rules {
        RuleSetId::Quake | RuleSetId::QuakeWorld => None,
        RuleSetId::Quake2 => NonZeroU32::new(100),
        RuleSetId::Quake2Rerelease => NonZeroU32::new(25),
        // Preserve the host's bounded one-ms minimum for rates above 1000.
        RuleSetId::Quake3 => NonZeroU32::new((1000 / sv_fps.get()).max(1)),
    }
}

/// Q3 InsertLinkAfter puts the newest entity first; Q1/QW/Q2 append it.
pub fn link_first(rules: RuleSetId) -> bool {
    rules == RuleSetId::Quake3
}

pub fn link_order(rules: RuleSetId) -> LinkOrder {
    if link_first(rules) {
        LinkOrder::Head
    } else {
        LinkOrder::Tail
    }
}
