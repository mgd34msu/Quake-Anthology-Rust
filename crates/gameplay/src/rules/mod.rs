pub mod appearance;
pub mod movement;
pub mod presentation;
pub mod q1;
pub mod q2;
pub mod q3;
mod sound;
pub use sound::sound_policy;

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

/// Native frame-driven duration policy; explicit fixed-rate providers keep
/// their caller-selected cadence.
#[derive(Clone, Copy)]
pub struct FrameDuration {
    pub bounds: [f64; 2],
    pub override_cvar: &'static str,
}
pub fn frame_duration(rules: RuleSetId) -> Option<FrameDuration> {
    (rules == RuleSetId::Quake).then_some(FrameDuration {
        bounds: [0.001, 0.1],
        override_cvar: "host_framerate",
    })
}

/// Native SERVER clock arithmetic, selected independently of module format.
#[derive(Clone, Copy)]
pub struct FrameClock {
    pub accumulate: bool,
    pub narrow_span: bool,
    pub advance_before: bool,
}
pub const fn frame_clock(rules: RuleSetId) -> FrameClock {
    FrameClock {
        accumulate: matches!(rules, RuleSetId::Quake | RuleSetId::QuakeWorld),
        narrow_span: matches!(rules, RuleSetId::QuakeWorld),
        advance_before: matches!(rules, RuleSetId::QuakeWorld),
    }
}

/// QC map inhibition is a module rule, independent of the map's file syntax.
pub fn map_inhibit_flags(rules: RuleSetId, deathmatch: bool, skill: i32) -> i32 {
    if rules == RuleSetId::QuakeWorld || deathmatch {
        2048
    } else {
        match skill {
            0 => 256,
            1 => 512,
            _ => 1024,
        }
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
