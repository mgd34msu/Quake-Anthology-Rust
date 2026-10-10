//! One table walker for native scalar deltas. Protocol records supply words;
//! only these field descriptors select comparison, projection and presence.
use crate::message::{Error, ErrorKind, Reader, Writer};

#[derive(Clone, Copy)]
pub(crate) enum Value {
    Unsigned,
    Signed,
    Angle16,
    Time,
    ZeroUnsigned,
    Float { zero: bool },
}
#[derive(Clone, Copy)]
pub(crate) struct Field {
    pub word: usize,
    pub bits: u8,
    pub flag: u32,
    pub value: Value,
}
impl Field {
    fn equal(self, from: u32, to: u32) -> bool {
        match self.value {
            Value::Angle16 => f32::from_bits(from) == f32::from_bits(to),
            _ => from == to,
        }
    }
    fn project(self, word: u32) -> u32 {
        match self.value {
            Value::Angle16 => (f32::from_bits(word) * 65536.0 / 360.0) as i32 as u32,
            _ => word,
        }
    }
    fn restore(self, word: u32) -> u32 {
        match self.value {
            Value::Signed => ((word << (32 - self.bits)) as i32 >> (32 - self.bits)) as u32,
            Value::Angle16 => ((word as i16 as f32) * (360.0 / 65536.0)).to_bits(),
            _ => word,
        }
    }
}
pub(crate) enum Presence {
    Fixed,
    Mask(u8),
    OptionalMask(u8),
    LastChanged(u8),
    Changed {
        aggregate: bool,
        keyed: bool,
        key_extra: u8,
    },
}
/// Static table metadata selects extended state values at compile time. Command
/// tables eliminate float/zero-shortcut branches from the same scalar walker.
pub(crate) struct Group<const STATE: bool = false> {
    pub fields: &'static [Field],
    pub presence: Presence,
}
pub(crate) fn changed(fields: &[Field], from: &[u32], to: &[u32]) -> usize {
    fields
        .iter()
        .rposition(|f| !f.equal(from[f.word], to[f.word]))
        .map_or(0, |i| i + 1)
}

/// Static engine tables consume the fixed records supplied by the projections.
/// A malformed wire message affects only the caller's temporary decoded record.
#[inline]
pub(crate) fn write<const STATE: bool>(
    groups: &[Group<STATE>],
    from: &[u32],
    to: &[u32],
    key: u32,
    writer: &mut Writer<'_>,
) -> Result<(), Error> {
    for group in groups {
        let masked = matches!(
            group.presence,
            Presence::Mask(_) | Presence::OptionalMask(_)
        );
        let compare = matches!(group.presence, Presence::Changed { .. })
            || STATE && matches!(group.presence, Presence::LastChanged(_));
        let field_key = if matches!(group.presence, Presence::Changed { keyed: true, .. }) {
            key
        } else {
            0
        };
        let mut mask = 0;
        let mut count = group.fields.len();
        match group.presence {
            Presence::Mask(bits) | Presence::OptionalMask(bits) => {
                for field in group.fields {
                    if !field.equal(from[field.word], to[field.word]) {
                        mask |= field.flag;
                    }
                }
                if matches!(group.presence, Presence::OptionalMask(_)) {
                    writer.write_bits(u32::from(mask != 0), 1)?;
                    if mask == 0 {
                        continue;
                    }
                }
                writer.write_bits(mask, bits)?;
            }
            Presence::LastChanged(bits) if STATE => {
                count = changed(group.fields, from, to);
                writer.write_bits(count as u32, bits)?;
            }
            Presence::Changed {
                aggregate: true, ..
            } => {
                let changed = group
                    .fields
                    .iter()
                    .any(|f| !f.equal(from[f.word], to[f.word]));
                writer.write_bits(u32::from(changed), 1)?;
                if !changed {
                    continue;
                }
            }
            _ => {}
        }
        for field in &group.fields[..count] {
            let old = from[field.word];
            let new = to[field.word];
            if masked && field.flag != 0 && mask & field.flag == 0 {
                continue;
            }
            if compare {
                let changed = !field.equal(old, new);
                writer.write_bits(u32::from(changed), 1)?;
                if !changed {
                    continue;
                }
            }
            if let Value::Time = field.value {
                let difference = new.wrapping_sub(old) as i32;
                let short = difference < 256;
                writer.write_bits(u32::from(short), 1)?;
                writer.write_bits(
                    if short { difference as u32 } else { new },
                    if short { 8 } else { 32 },
                )?;
            } else if STATE && let Value::Float { zero } = field.value {
                let value = f32::from_bits(new);
                if zero {
                    writer.write_bits(u32::from(value != 0.0), 1)?;
                    if value == 0.0 {
                        continue;
                    }
                }
                let integer = value as i32;
                let small = (-4096..4096).contains(&integer) && integer as f32 == value;
                writer.write_bits(u32::from(!small), 1)?;
                writer.write_bits(
                    if small { (integer + 4096) as u32 } else { new },
                    if small { 13 } else { 32 },
                )?;
            } else if STATE && let Value::ZeroUnsigned = field.value {
                writer.write_bits(u32::from(new != 0), 1)?;
                if new != 0 {
                    writer.write_bits(new, field.bits)?;
                }
            } else {
                writer.write_bits(field.project(new) ^ field_key, field.bits)?;
            }
        }
    }
    Ok(())
}
#[inline(always)]
pub(crate) fn read<const STATE: bool>(
    groups: &[Group<STATE>],
    words: &mut [u32],
    key: u32,
    reader: &mut Reader<'_>,
) -> Result<(), Error> {
    for group in groups {
        let masked = matches!(
            group.presence,
            Presence::Mask(_) | Presence::OptionalMask(_)
        );
        let compare = matches!(group.presence, Presence::Changed { .. })
            || STATE && matches!(group.presence, Presence::LastChanged(_));
        let (keyed, key_extra) = match group.presence {
            Presence::Changed {
                keyed, key_extra, ..
            } => (keyed, key_extra),
            _ => (false, 0),
        };
        let mut count = group.fields.len();
        let mask = match group.presence {
            Presence::Mask(bits) => reader.read_bits(bits)?,
            Presence::OptionalMask(bits) => {
                if reader.read_bits(1)? == 0 {
                    continue;
                }
                reader.read_bits(bits)?
            }
            Presence::LastChanged(bits) if STATE => {
                count = reader.read_bits(bits)? as usize;
                if count > group.fields.len() {
                    return Err(Error {
                        byte: reader.byte_position(),
                        kind: ErrorKind::Width,
                    });
                }
                0
            }
            Presence::Changed {
                aggregate: true, ..
            } => {
                if reader.read_bits(1)? == 0 {
                    continue;
                }
                0
            }
            _ => 0,
        };
        for field in &group.fields[..count] {
            let mut field_key = 0;
            if masked && field.flag != 0 && mask & field.flag == 0 {
                continue;
            }
            if compare && reader.read_bits(1)? == 0 {
                continue;
            }
            if keyed {
                field_key = key & (u32::MAX >> (32 - (field.bits + key_extra).min(32)));
            }
            words[field.word] = if let Value::Time = field.value {
                if reader.read_bits(1)? != 0 {
                    words[field.word].wrapping_add(reader.read_bits(8)?)
                } else {
                    reader.read_bits(32)?
                }
            } else if STATE && let Value::Float { zero } = field.value {
                if zero && reader.read_bits(1)? == 0 {
                    0
                } else if reader.read_bits(1)? == 0 {
                    ((reader.read_bits(13)? as i32 - 4096) as f32).to_bits()
                } else {
                    reader.read_bits(32)?
                }
            } else if STATE && let Value::ZeroUnsigned = field.value {
                if reader.read_bits(1)? == 0 {
                    0
                } else {
                    reader.read_bits(field.bits)?
                }
            } else {
                field.restore(reader.read_bits(field.bits)? ^ field_key)
            };
        }
    }
    Ok(())
}
