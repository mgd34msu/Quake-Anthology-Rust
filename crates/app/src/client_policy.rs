//! Built-in walk-through client policy; this is not a loaded guest module.
use qa_console::{catalog::Scope, cvars::Cvars, views::Context};
use qa_core::primitives::RuleSetId;
use qa_session::timing::TickRate;
use std::num::NonZeroU32;

pub use qa_core::primitives::ClientPolicy;

/// Startup role choices are explicit and independent for each local seat.
pub fn parse_seat(
    value: &str,
) -> Result<(qa_core::sys_events::SeatId, ClientPolicy), &'static str> {
    crate::options::parse_seat_value(value, ClientPolicy::parse)
}

pub fn tick_rate(rules: RuleSetId, cvars: &mut Cvars) -> Result<TickRate, &'static str> {
    let fps = if rules == RuleSetId::Quake3 {
        let handle = cvars.find("sv_fps").ok_or("missing sv_fps")?;
        let mut value = cvars.integer_in(handle, rules);
        if value < 1 {
            let view = cvars
                .bind(
                    "sv_fps",
                    Context {
                        source: rules,
                        side: Scope::Server,
                        ..cvars.context()
                    },
                )
                .ok_or("missing Q3 sv_fps view")?;
            cvars
                .write(view, "10")
                .map_err(|_| "Q3 sv_fps fallback write")?;
            value = 10;
        }
        NonZeroU32::new(value as u32).ok_or("invalid Q3 sv_fps")?
    } else {
        NonZeroU32::MIN
    };
    Ok(match qa_gameplay::rules::tick_millis(rules, fps) {
        Some(period) => TickRate::FixedMilliseconds(period),
        None => TickRate::FrameDriven,
    })
}
