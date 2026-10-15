//! Built-in walk-through client policy; this is not a loaded guest module.
use qa_console::{catalog::Scope, cvars::Cvars, views::Context};
use qa_core::primitives::RuleSetId;
use qa_session::timing::TickRate;
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientPolicy {
    pub client: RuleSetId,
    pub input: RuleSetId,
    pub movement: RuleSetId,
    pub trace: RuleSetId,
}

impl ClientPolicy {
    /// Startup role choices are explicit and independent for each local seat.
    pub fn parse_seat(value: &str) -> Result<(qa_core::sys_events::SeatId, Self), &'static str> {
        crate::options::parse_seat_value(value, Self::parse)
    }
    fn parse(value: &str) -> Option<Self> {
        let mut fields = value.split(':');
        let client = RuleSetId::parse(fields.next()?)?;
        let movement = RuleSetId::parse(fields.next()?)?;
        let trace = RuleSetId::parse(fields.next()?)?;
        let input = fields
            .next()
            .map(RuleSetId::parse)
            .unwrap_or(Some(client))?;
        if fields.next().is_some() {
            return None;
        }
        Some(Self {
            client,
            input,
            movement,
            trace,
        })
    }

    pub fn select(
        explicit_client: Option<RuleSetId>,
        stock_client: Option<RuleSetId>,
        explicit_movement: Option<RuleSetId>,
        explicit_trace: Option<RuleSetId>,
        explicit_input: Option<RuleSetId>,
    ) -> Result<Self, &'static str> {
        let client = explicit_client
            .or(stock_client)
            .ok_or("unknown or ambiguous product needs --client-module q1/qw/q2/q2rr/q3")?;
        Ok(Self {
            client,
            input: explicit_input.unwrap_or(client),
            movement: explicit_movement.unwrap_or(client),
            trace: explicit_trace.unwrap_or(client),
        })
    }
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
