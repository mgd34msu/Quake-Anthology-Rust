//! One table walker for native scalar deltas. Protocol records supply words;
//! only these field descriptors select comparison, projection and presence.
use crate::message::{Error, Reader, Writer};

#[derive(Clone, Copy)]
pub(crate) enum Value {
    Unsigned,
    Signed,
    Angle16,
    Time,
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
    Changed {
        aggregate: bool,
        keyed: bool,
        key_extra: u8,
    },
}
pub(crate) struct Group {
    pub fields: &'static [Field],
    pub presence: Presence,
}

/// Static engine tables consume the fixed records supplied by the projections.
/// A malformed wire message affects only the caller's temporary decoded record.
pub(crate) fn write(
    groups: &[Group],
    from: &[u32],
    to: &[u32],
    key: u32,
    writer: &mut Writer<'_>,
) -> Result<(), Error> {
    for group in groups {
        let mut mask = 0;
        match group.presence {
            Presence::Mask(bits) => {
                for field in group.fields {
                    if !field.equal(from[field.word], to[field.word]) {
                        mask |= field.flag;
                    }
                }
                writer.write_bits(mask, bits)?;
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
        for field in group.fields {
            let old = from[field.word];
            let new = to[field.word];
            let mut field_key = 0;
            match group.presence {
                Presence::Mask(_) if field.flag != 0 && mask & field.flag == 0 => continue,
                Presence::Changed { keyed, .. } => {
                    let changed = !field.equal(old, new);
                    writer.write_bits(u32::from(changed), 1)?;
                    if !changed {
                        continue;
                    }
                    if keyed {
                        field_key = key;
                    }
                }
                _ => {}
            }
            if let Value::Time = field.value {
                let difference = new.wrapping_sub(old) as i32;
                let short = difference < 256;
                writer.write_bits(u32::from(short), 1)?;
                writer.write_bits(
                    if short { difference as u32 } else { new },
                    if short { 8 } else { 32 },
                )?;
            } else {
                writer.write_bits(field.project(new) ^ field_key, field.bits)?;
            }
        }
    }
    Ok(())
}
pub(crate) fn read(
    groups: &[Group],
    words: &mut [u32],
    key: u32,
    reader: &mut Reader<'_>,
) -> Result<(), Error> {
    for group in groups {
        let mask = match group.presence {
            Presence::Mask(bits) => reader.read_bits(bits)?,
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
        for field in group.fields {
            let mut field_key = 0;
            match group.presence {
                Presence::Mask(_) if field.flag != 0 && mask & field.flag == 0 => continue,
                Presence::Changed {
                    keyed, key_extra, ..
                } => {
                    if reader.read_bits(1)? == 0 {
                        continue;
                    }
                    if keyed {
                        field_key = key & (u32::MAX >> (32 - (field.bits + key_extra).min(32)));
                    }
                }
                _ => {}
            }
            words[field.word] = if let Value::Time = field.value {
                if reader.read_bits(1)? != 0 {
                    words[field.word].wrapping_add(reader.read_bits(8)?)
                } else {
                    reader.read_bits(32)?
                }
            } else {
                field.restore(reader.read_bits(field.bits)? ^ field_key)
            };
        }
    }
    Ok(())
}
