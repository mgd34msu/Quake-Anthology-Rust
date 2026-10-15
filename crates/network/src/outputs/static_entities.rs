//! Native persistent entities use the same baseline fields as dynamic entities.
use super::Error;
use crate::{
    commands::packet::Protocol,
    message::{Encoding, Reader, Writer},
    projection::EntityProjection,
    states,
};
use qa_core::primitives::{EntityRecord, ModelRef, ResourceNamespace};

pub const STATIC_ENTITY_BYTES: usize = 14;

pub fn read_static_entity(
    protocol: Protocol,
    bytes: &[u8],
    namespace: ResourceNamespace,
) -> Result<(EntityRecord, usize), Error> {
    if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
        return Err(Error::Unsupported);
    }
    if bytes.first().copied().ok_or(Error::Truncated)? != 20 {
        return Err(Error::Opcode);
    }
    let mut reader = Reader::new(&bytes[1..], Encoding::Bytes);
    let baseline = states::read_q1_baseline(&mut reader)?;
    let mut words = [0; states::QW_ENTITY_WORDS];
    words[..states::NQ_ENTITY_WORDS].copy_from_slice(&baseline);
    let mut record = EntityRecord::default();
    if !EntityProjection::load(protocol).apply(&words, &mut record, namespace) {
        return Err(Error::Unsupported);
    }
    Ok((record, 1 + reader.byte_position()))
}

pub fn static_entity(
    protocol: Protocol,
    record: &EntityRecord,
    mut resolve_model: impl FnMut(ModelRef) -> Option<u32>,
    output: &mut [u8],
) -> Result<usize, Error> {
    if !matches!(protocol, Protocol::NetQuake15 | Protocol::QuakeWorld28) {
        return Err(Error::Unsupported);
    }
    let model = match record.appearance.models[0] {
        Some(model) => Some(
            resolve_model(model)
                .filter(|&n| n <= 255)
                .ok_or(Error::Unsupported)?,
        ),
        None => None,
    };
    let mut words = [0; states::QW_ENTITY_WORDS];
    if !EntityProjection::load(protocol).reduce(
        record,
        &crate::projection::EntityContext::default(),
        &mut words,
        |_| model,
        |_| None,
        |reference| Some(reference.ordinal()),
    ) {
        return Err(Error::Unsupported);
    }
    let mut writer = Writer::new(output, Encoding::Bytes);
    writer.write_bits(20, 8)?;
    states::write_q1_baseline(&mut writer, &words, protocol == Protocol::NetQuake15)?;
    Ok(writer.size())
}
