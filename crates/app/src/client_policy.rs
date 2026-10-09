//! Built-in walk-through client policy; this is not a loaded guest module.
use qa_console::{catalog::Scope, cvars::Cvars, views::Context};
use qa_core::primitives::RuleSetId;
use qa_session::timing::TickRate;
use qa_world::area::LinkOrder;
use std::num::NonZeroU32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClientPolicy {
    pub client: RuleSetId,
    pub movement: RuleSetId,
    pub trace: RuleSetId,
}

impl ClientPolicy {
    /// Startup role choices are explicit and independent for each local seat.
    pub fn parse_seat(value: &str) -> Result<(qa_core::sys_events::SeatId, Self), &'static str> {
        let mut fields = value.split(':');
        let seat = fields
            .next()
            .and_then(|value| value.parse().ok())
            .and_then(qa_core::sys_events::SeatId::new)
            .ok_or("seat-policy needs a seat in 0..4")?;
        let mut rule = || {
            fields
                .next()
                .and_then(RuleSetId::parse)
                .ok_or("seat-policy needs seat:client:movement:trace (q1/qw/q2/q2rr/q3)")
        };
        let policy = Self {
            client: rule()?,
            movement: rule()?,
            trace: rule()?,
        };
        if fields.next().is_some() {
            return Err("seat-policy has extra fields");
        }
        Ok((seat, policy))
    }

    pub fn select(
        explicit_client: Option<RuleSetId>,
        stock_client: Option<RuleSetId>,
        explicit_movement: Option<RuleSetId>,
        explicit_trace: Option<RuleSetId>,
    ) -> Result<Self, &'static str> {
        let client = explicit_client
            .or(stock_client)
            .ok_or("unknown or ambiguous product needs --client-module q1/qw/q2/q2rr/q3")?;
        Ok(Self {
            client,
            movement: explicit_movement.unwrap_or(client),
            trace: explicit_trace.unwrap_or(client),
        })
    }

    pub fn tick_rate(self, cvars: &mut Cvars) -> Result<TickRate, &'static str> {
        let fps = if self.client == RuleSetId::Quake3 {
            let handle = cvars.find("sv_fps").ok_or("missing sv_fps")?;
            let mut value = cvars.integer_in(handle, self.client);
            if value < 1 {
                let view = cvars
                    .bind(
                        "sv_fps",
                        Context {
                            source: self.client,
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
        Ok(match qa_gameplay::rules::tick_millis(self.client, fps) {
            Some(period) => TickRate::FixedMilliseconds(period),
            None => TickRate::FrameDriven,
        })
    }

    pub fn link_order(self) -> LinkOrder {
        if qa_gameplay::rules::link_first(self.client) {
            LinkOrder::Head
        } else {
            LinkOrder::Tail
        }
    }
}
