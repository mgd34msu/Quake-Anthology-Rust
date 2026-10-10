//! Bounded prepared-packet submission for the current channel consumers.
use qa_core::sys_events::EventTime;
use qa_network::channel::{Channel, Unreliable};

#[derive(Default)]
pub(crate) struct Submission {
    pub packets: u64,
    pub blocked: bool,
    pub complete: bool,
}

pub(crate) fn send<T>(
    channel: &mut Channel,
    time: EventTime,
    context: &mut T,
    mut prepare: impl FnMut(&mut Channel, &mut T) -> bool,
    mut admit: impl FnMut(&[u8]) -> bool,
    mut submitted: impl FnMut(&Channel, &mut T, Unreliable) -> bool,
) -> Submission {
    let mut result = Submission::default();
    // The load-sized control ring plus one ordinary payload. Rejected bytes
    // retain their prepared state; this never polls a physical input source.
    for _ in 0..17 {
        if channel.pending_packet().is_none() && !prepare(channel, context) {
            break;
        }
        let Ok(Some(disposition)) = channel.submit_with(time, &mut admit) else {
            result.blocked = true;
            break;
        };
        result.packets += 1;
        if submitted(channel, context, disposition) {
            result.complete = true;
            break;
        }
    }
    result
}
