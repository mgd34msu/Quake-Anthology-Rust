use crate::transport::{self, Submission};
use qa_core::{
    events::{OutputSubmission, TextStore},
    primitives::{NativeDelivery, PrintEvent, PrintText},
    sys_events::EventTime,
};
use qa_network::{
    channel::{Channel, TransmitError, Unreliable},
    ingress::Connection,
};

pub(super) fn transmit(
    connection: &mut Connection,
    payload: Option<(u64, &[u8])>,
    time: EventTime,
    admit: impl FnMut(&[u8]) -> bool,
) -> Result<(Option<OutputSubmission>, Submission), TransmitError> {
    if connection.pending_output.is_some()
        && connection.channel.pending_packet().is_none()
        && !connection.channel.pending_fragments()
    {
        // QW reply alignment can cancel an unsent packet. Its ring record
        // remains unsent and must be encoded again, not completed by a new
        // empty control reply.
        connection.pending_output = None;
        if payload.is_none() {
            return Ok((None, Submission::default()));
        }
    }
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
    if protocol == qa_network::commands::packet::Protocol::Quake3_68
        && qa_network::outputs::read_native_service(
            qa_network::commands::packet::Protocol::native(delivery.source),
            bytes,
        )
        .is_ok_and(|(service, length)| {
            length == bytes.len() && matches!(service, qa_network::outputs::NativeService::Nop)
        })
    {
        return Ok((OutputSubmission::BestEffort, Submission::default(), 0));
    }
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
        return reliable(&mut connection.channel, bytes)
            .map(|submission| (submission, Submission::default()));
    }
    let (disposition, submitted) =
        transmit(connection, Some((sequence, bytes)), time, admit).map_err(|_| ())?;
    Ok((disposition.unwrap_or(OutputSubmission::Unsent), submitted))
}

pub(super) fn reliable(channel: &mut Channel, bytes: &[u8]) -> Result<OutputSubmission, ()> {
    match channel.queue_reliable(bytes) {
        Ok(receipt) => Ok(OutputSubmission::Reliable(receipt)),
        Err(TransmitError::Full) => Ok(OutputSubmission::Unsent),
        Err(_) => Err(()),
    }
}

/// One print record produces one native service or one Q3 reliable command.
#[expect(
    clippy::too_many_arguments,
    reason = "Print text borrows the existing ring and fixed transport scratch."
)]
pub(super) fn submit_print(
    connection: &mut Connection,
    sequence: u64,
    delivery: Option<NativeDelivery>,
    print: PrintEvent,
    texts: &TextStore,
    input: &mut [u8],
    output: &mut [u8],
    time: EventTime,
    primary: bool,
    admit: impl FnMut(&[u8]) -> bool,
) -> Result<(OutputSubmission, Submission), ()> {
    let protocol = connection.commands.as_ref().ok_or(())?.protocol;
    let (text, source) = match print.text {
        PrintText::Text(id) => (texts.get(id).ok_or(())?, None),
        PrintText::Native {
            wire,
            text,
            consumed,
        } => {
            let delivery = delivery.ok_or(())?;
            let bytes = texts.copy_payload(wire, input).ok_or(())?;
            let text = bytes.get(text.range()).ok_or(())?;
            (
                text,
                Some(qa_network::outputs::PrintSource {
                    source: delivery.source,
                    primary,
                    wire: &bytes[..wire.len()],
                    consumed: consumed as usize,
                }),
            )
        }
    };
    let length =
        qa_network::outputs::print(protocol, print.kind, print.level, text, source, output)
            .map_err(|_| ())?;
    let bytes = &output[..length];
    if delivery.is_none_or(|delivery| delivery.reliable)
        || protocol == qa_network::commands::packet::Protocol::Quake3_68
    {
        reliable(&mut connection.channel, bytes)
            .map(|submission| (submission, Submission::default()))
    } else {
        submit_encoded(
            connection,
            sequence,
            delivery.ok_or(())?,
            bytes,
            time,
            admit,
        )
    }
}
