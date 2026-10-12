//! Configuration records retain their ring text until native channel delivery.
use qa_core::events::OutputSubmission;
use qa_network::{
    channel::{Channel, TransmitError},
    commands::packet::Protocol,
};

pub(super) fn submit(
    channel: &mut Channel,
    protocol: Protocol,
    sequence: u64,
    index: u32,
    text: &[u8],
    pending: &mut Option<(u64, usize)>,
) -> Result<OutputSubmission, ()> {
    let target = protocol.config_layout().ok_or(())?;
    let index = index as usize;
    if text.len() > target.capacity(index).ok_or(())? {
        return Err(());
    }
    let mut bytes = [0; 8196];
    if protocol != Protocol::Quake3_68 {
        let length = qa_network::outputs::configstring(protocol, index as u32, text, &mut bytes)
            .map_err(|_| ())?;
        return match channel.queue_reliable(&bytes[..length]) {
            Ok(receipt) => Ok(OutputSubmission::Reliable(receipt)),
            Err(TransmitError::Full) => Ok(OutputSubmission::Unsent),
            Err(_) => Err(()),
        };
    }
    let mut sent = pending
        .filter(|(record, _)| *record == sequence)
        .map_or(0, |(_, sent)| sent);
    loop {
        let chunk = qa_network::outputs::config_command(index as u32, text, sent, &mut bytes)
            .map_err(|_| ())?;
        match channel.queue_reliable(&bytes[..chunk.length]) {
            Ok(receipt) => {
                sent = chunk.next;
                if chunk.complete {
                    *pending = None;
                    // Native command ACKs are cumulative: the final receipt
                    // retires every earlier chunk of this retained record.
                    return Ok(OutputSubmission::Reliable(receipt));
                }
                *pending = Some((sequence, sent));
            }
            Err(TransmitError::Full) => return Ok(OutputSubmission::Unsent),
            Err(_) => return Err(()),
        }
    }
}
