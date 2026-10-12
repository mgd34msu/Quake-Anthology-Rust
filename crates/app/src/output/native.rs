use crate::transport::{self, Submission};
use qa_core::{events::OutputSubmission, primitives::NativeDelivery, sys_events::EventTime};
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
    let mut dropped = false;
    let mut submitted = transport::send(
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
            if disposition == Unreliable::Included {
                *pending = Some(sequence);
            }
            true
        },
        admit,
        // Included describes the initial encoding, not completion of a
        // fragmented send. Continuations report Deferred, including the last
        // fragment; the channel owns the actual completion boundary.
        |channel, pending, disposition| {
            if disposition == Unreliable::Dropped {
                dropped = true;
                true
            } else {
                pending.is_some() && !channel.pending_fragments()
            }
        },
    );
    let retired = submitted.complete;
    // A reliable/control packet may have been submitted while its optional
    // native payload was dropped. That retires this record but is not delivery.
    if dropped {
        submitted.complete = false;
        submitted.dropped = true;
    }
    if rejected.is_some() || retired {
        connection.pending_output = None;
    }
    if let Some(error) = rejected {
        return Err(error);
    }
    let disposition = retired.then_some(OutputSubmission::BestEffort);
    Ok((disposition, submitted))
}

pub(super) fn submit(
    connection: &mut Connection,
    sequence: u64,
    delivery: NativeDelivery,
    bytes: &[u8],
    output: &mut [u8],
    time: EventTime,
    primary: bool,
    project: impl FnMut(
        qa_network::outputs::Cue,
        usize,
    ) -> Result<qa_network::outputs::Cue, qa_network::outputs::Error>,
    admit: impl FnMut(&[u8]) -> bool,
) -> Result<(OutputSubmission, Submission, u64), ()> {
    let protocol = connection.commands.as_ref().ok_or(())?.protocol;
    let encoded = match qa_network::outputs::native(
        protocol,
        delivery.source,
        bytes,
        output,
        primary,
        project,
    ) {
        Ok(encoded) => encoded,
        Err(qa_network::outputs::Error::Pending) => {
            return Ok((OutputSubmission::Unsent, Submission::default(), 0));
        }
        Err(_) => return Err(()),
    };
    let bytes = &output[..encoded.length];
    let (disposition, submitted) =
        submit_encoded(connection, sequence, delivery, bytes, time, admit)?;
    Ok((
        disposition,
        submitted,
        if disposition != OutputSubmission::Unsent || connection.pending_output == Some(sequence) {
            encoded.unsupported
        } else {
            0
        },
    ))
}

pub(super) fn submit_encoded(
    connection: &mut Connection,
    sequence: u64,
    delivery: NativeDelivery,
    bytes: &[u8],
    time: EventTime,
    admit: impl FnMut(&[u8]) -> bool,
) -> Result<(OutputSubmission, Submission), ()> {
    if bytes.is_empty() {
        return Ok((OutputSubmission::BestEffort, Submission::default()));
    }
    if delivery.reliable {
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
