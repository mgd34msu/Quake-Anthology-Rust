use crate::transport::{self, Submission};
use qa_core::{events::OutputSubmission, primitives::NativeMessage, sys_events::EventTime};
use qa_network::{
    channel::{TransmitError, Unreliable},
    ingress::Connection,
};

pub(super) fn transmit(
    connection: &mut Connection,
    payload: Option<(u64, &[u8])>,
    time: EventTime,
    admit: impl FnMut(&[u8]) -> bool,
) -> Result<(Option<OutputSubmission>, Submission), TransmitError> {
    let Some(sequence) = connection
        .pending_output
        .or(payload.map(|(sequence, _)| sequence))
    else {
        return Ok((None, Submission::default()));
    };
    // A snapshot or ordinary channel control retains its existing sender.
    if connection.pending_output.is_none()
        && (connection.channel.pending_packet().is_some() || connection.channel.pending_fragments())
    {
        return Ok((None, Submission::default()));
    }
    let mut rejected = None;
    let submitted = transport::send(
        &mut connection.channel,
        time,
        &mut connection.pending_output,
        |channel, pending| {
            let body = if channel.pending_fragments() {
                None
            } else {
                payload.map(|(_, bytes)| bytes)
            };
            let packet = match channel.prepare(body, time) {
                Ok(Some(packet)) => packet,
                Ok(None) => return false,
                Err(error) => {
                    rejected = Some(error);
                    return false;
                }
            };
            let disposition = packet.unreliable;
            if disposition != Unreliable::Deferred || channel.pending_fragments() {
                *pending = Some(sequence);
            }
            true
        },
        admit,
        // Included describes the initial encoding, not completion of a
        // fragmented send. Continuations report Deferred, including the last
        // fragment; the channel owns the actual completion boundary.
        |channel, pending, _| pending.is_some() && !channel.pending_fragments(),
    );
    if rejected.is_some() || submitted.complete {
        connection.pending_output = None;
    }
    if let Some(error) = rejected {
        return Err(error);
    }
    let disposition = submitted.complete.then_some(OutputSubmission::BestEffort);
    Ok((disposition, submitted))
}

pub(super) fn submit(
    connection: &mut Connection,
    sequence: u64,
    message: NativeMessage,
    bytes: &[u8],
    time: EventTime,
    admit: impl FnMut(&[u8]) -> bool,
) -> Result<(OutputSubmission, Submission), ()> {
    let protocol = connection.commands.as_ref().ok_or(())?.protocol;
    let bytes =
        qa_network::outputs::native(protocol, message.delivery.source, bytes).map_err(|_| ())?;
    if message.delivery.reliable {
        return match connection.channel.queue_reliable(bytes) {
            Ok(receipt) => Ok((OutputSubmission::Reliable(receipt), Submission::default())),
            Err(qa_network::channel::TransmitError::Full) => {
                Ok((OutputSubmission::Unsent, Submission::default()))
            }
            Err(_) => Err(()),
        };
    }
    let (disposition, submitted) =
        transmit(connection, Some((sequence, bytes)), time, admit).map_err(|_| ())?;
    Ok((disposition.unwrap_or(OutputSubmission::Unsent), submitted))
}
