//! One table walker for native scalar deltas. Protocol records supply words;
//! only these field descriptors select comparison, projection and presence.
use crate::message::{Error, ErrorKind, Reader, Writer};
use qa_core::math::{AngleShortForm, angle_to_short, short_to_angle};

#[derive(Clone, Copy)]
pub(crate) enum ScaleRead {
    Signed,
    Unsigned,
    UnsignedDivide,
    SignedTenthsDelta,
    SignedInverse,
}
#[derive(Clone, Copy)]
pub(crate) enum Packed {
    Unsigned16,
    Unsigned15,
    Signed15,
    Signed8,
}
impl Packed {
    fn width(self, word: u32) -> u8 {
        let signed = matches!(self, Self::Signed15 | Self::Signed8);
        if if signed {
            (word as i32) < 256
        } else {
            word < 256
        } {
            8
        } else if match self {
            Self::Unsigned16 => word < 0x10000,
            Self::Unsigned15 => word < 0x8000,
            Self::Signed15 => (word as i32) < 0x8000,
            Self::Signed8 => true,
        } {
            16
        } else {
            32
        }
    }
}
#[derive(Clone, Copy)]
pub(crate) enum Value {
    Unsigned,
    Signed,
    Angle16,
    Time,
    ZeroUnsigned,
    Float { zero: bool },
    Scaled { factor: u8, read: ScaleRead },
    Angle8 { integral: bool },
    Packed(Packed),
    Transient,
    FloatInt,
}
#[derive(Clone, Copy)]
pub(crate) struct Field {
    pub word: usize,
    bits: u8,
    pub flag: u32,
    pub value: Value,
}
impl Field {
    pub(crate) const fn new(word: usize, bits: u8, flag: u32, value: Value) -> Self {
        Self {
            word,
            bits,
            flag,
            value,
        }
    }
    fn equal<const STATE: bool, const PREFIX: bool, const PACKED: bool, const FLOAT_BYTES: bool>(
        self,
        from: u32,
        to: u32,
    ) -> bool {
        match self.value {
            Value::FloatInt if FLOAT_BYTES => (from as i32 as f32) == f32::from_bits(to),
            Value::Transient if PACKED => to == 0,
            Value::Angle16 => f32::from_bits(from) == f32::from_bits(to),
            Value::Angle8 { .. } if PREFIX => f32::from_bits(from) == f32::from_bits(to),
            Value::Scaled {
                read: ScaleRead::SignedTenthsDelta,
                ..
            } if PREFIX => {
                let difference = f64::from(f32::from_bits(to) - f32::from_bits(from));
                // Unordered native comparisons leave the origin unchanged.
                !(difference < -0.1 || difference > 0.1)
            }
            Value::Scaled { .. } if STATE => f32::from_bits(from) == f32::from_bits(to),
            _ => from == to,
        }
    }
    fn flags<const PACKED: bool>(self, word: u32) -> u32 {
        if PACKED && let Value::Packed(rule) = self.value {
            let bits = rule.width(word);
            if self.bits != 0 {
                return if self.bits == bits { self.flag } else { 0 };
            }
            let byte = self.flag & self.flag.wrapping_neg();
            match bits {
                8 => byte,
                16 => self.flag ^ byte,
                _ => self.flag,
            }
        } else {
            self.flag
        }
    }
    fn width<const PACKED: bool>(self, flags: u32) -> u8 {
        if PACKED && self.bits == 0 && matches!(self.value, Value::Packed(_)) {
            let selected = flags & self.flag;
            if selected == self.flag {
                32
            } else if selected == self.flag & self.flag.wrapping_neg() {
                8
            } else {
                16
            }
        } else {
            self.bits
        }
    }
    fn project<const STATE: bool, const PREFIX: bool, const FLOAT_BYTES: bool>(
        self,
        word: u32,
    ) -> u32 {
        match self.value {
            Value::Angle16 => {
                angle_to_short(f32::from_bits(word), AngleShortForm::MultiplyDivide) as u32
            }
            Value::Angle8 { integral } if PREFIX => {
                if FLOAT_BYTES && integral {
                    ((f32::from_bits(word) as i32).wrapping_mul(256) / 360) as u32
                } else {
                    (f32::from_bits(word) * 256.0 / 360.0) as i32 as u32
                }
            }
            Value::FloatInt if FLOAT_BYTES => f32::from_bits(word) as i32 as u32,
            Value::Scaled { factor, read } if STATE => {
                let value = f32::from_bits(word);
                if FLOAT_BYTES && matches!(read, ScaleRead::SignedInverse) {
                    (value / f32::from(factor)) as i32 as u32
                } else {
                    (value * f32::from(factor)) as i32 as u32
                }
            }
            _ => word,
        }
    }
    fn restore<
        const STATE: bool,
        const PREFIX: bool,
        const PACKED: bool,
        const FLOAT_BYTES: bool,
    >(
        self,
        word: u32,
        bits: u8,
    ) -> u32 {
        match self.value {
            Value::Packed(_) if PACKED => {
                if bits == 8 {
                    word
                } else {
                    ((word << (32 - bits)) as i32 >> (32 - bits)) as u32
                }
            }
            Value::FloatInt if FLOAT_BYTES && bits != 8 => {
                ((word << (32 - bits)) as i32 >> (32 - bits)) as u32
            }
            Value::Signed => ((word << (32 - bits)) as i32 >> (32 - bits)) as u32,
            Value::Angle16 => short_to_angle(i32::from(word as i16)).to_bits(),
            Value::Angle8 { .. } if PREFIX => ((word as i8 as f32) * (360.0 / 256.0)).to_bits(),
            Value::Scaled { factor, read } if STATE => {
                let value = if matches!(read, ScaleRead::Signed)
                    || PREFIX && matches!(read, ScaleRead::SignedTenthsDelta)
                    || FLOAT_BYTES && matches!(read, ScaleRead::SignedInverse)
                {
                    ((word << (32 - bits)) as i32 >> (32 - bits)) as f32
                } else {
                    word as f32
                };
                if FLOAT_BYTES && matches!(read, ScaleRead::SignedInverse) {
                    (value * f32::from(factor)).to_bits()
                } else if matches!(read, ScaleRead::UnsignedDivide) {
                    (value / f32::from(factor)).to_bits()
                } else {
                    (value * (1.0 / f32::from(factor))).to_bits()
                }
            }
            _ => word,
        }
    }
}
pub(crate) enum Presence {
    Fixed,
    Mask(u8),
    OptionalMask(u8),
    MaskPreset {
        bits: u8,
        always: u32,
        dependent_start: u8,
        dependent_count: u8,
    },
    LastChanged(u8),
    Changed {
        aggregate: bool,
        keyed: bool,
        key_extra: u8,
    },
}
/// Static table metadata selects extended state values and prefix-supplied
/// masks. Command and inline-mask tables eliminate unrelated branches from the
/// same scalar walker. A prefix table supplies its mask in the control word.
pub(crate) struct Group<
    const STATE: bool = false,
    const PREFIX: bool = false,
    const PACKED: bool = false,
    const FLOAT_BYTES: bool = false,
> {
    pub fields: &'static [Field],
    pub presence: Presence,
}
pub(crate) fn changed<
    const STATE: bool,
    const PREFIX: bool,
    const PACKED: bool,
    const FLOAT_BYTES: bool,
>(
    fields: &[Field],
    from: &[u32],
    to: &[u32],
) -> usize {
    fields
        .iter()
        .rposition(|f| !f.equal::<STATE, PREFIX, PACKED, FLOAT_BYTES>(from[f.word], to[f.word]))
        .map_or(0, |i| i + 1)
}
#[inline(always)]
pub(crate) fn mask<
    const STATE: bool,
    const PREFIX: bool,
    const PACKED: bool,
    const FLOAT_BYTES: bool,
>(
    fields: &[Field],
    from: &[u32],
    to: &[u32],
    always: u32,
    start: u8,
    count: u8,
) -> u32 {
    let mut mask = always;
    for (index, field) in fields.iter().enumerate() {
        if (!STATE
            || !(usize::from(start)..usize::from(start) + usize::from(count)).contains(&index))
            && !field.equal::<STATE, PREFIX, PACKED, FLOAT_BYTES>(from[field.word], to[field.word])
        {
            mask |= field.flags::<PACKED>(to[field.word]);
        }
    }
    mask
}

/// Static engine tables consume the fixed records supplied by the projections.
/// A malformed wire message affects only the caller's temporary decoded record.
#[inline(always)]
pub(crate) fn write<
    const STATE: bool,
    const PREFIX: bool,
    const PACKED: bool,
    const FLOAT_BYTES: bool,
>(
    groups: &[Group<STATE, PREFIX, PACKED, FLOAT_BYTES>],
    from: &[u32],
    to: &[u32],
    key: u32,
    writer: &mut Writer<'_>,
) -> Result<(), Error> {
    for group in groups {
        let masked = matches!(
            group.presence,
            Presence::Mask(_) | Presence::OptionalMask(_) | Presence::MaskPreset { .. }
        ) || PREFIX;
        let compare = matches!(group.presence, Presence::Changed { .. })
            || STATE && matches!(group.presence, Presence::LastChanged(_));
        let field_key = if matches!(group.presence, Presence::Changed { keyed: true, .. }) {
            key
        } else {
            0
        };
        let mut mask = if PREFIX { key } else { 0 };
        let mut count = group.fields.len();
        let mask_config = match group.presence {
            Presence::Mask(bits) | Presence::OptionalMask(bits) => Some((bits, 0, 0, 0)),
            Presence::MaskPreset {
                bits,
                always,
                dependent_start,
                dependent_count,
            } if STATE => Some((bits, always, dependent_start, dependent_count)),
            _ => None,
        };
        if let Some((bits, always, start, count)) = mask_config {
            mask = self::mask::<STATE, PREFIX, PACKED, FLOAT_BYTES>(
                group.fields,
                from,
                to,
                always,
                start,
                count,
            );
            if matches!(group.presence, Presence::OptionalMask(_)) {
                writer.write_bits(u32::from(mask != 0), 1)?;
                if mask == 0 {
                    continue;
                }
            }
            writer.write_bits(mask, bits)?;
        } else {
            match group.presence {
                Presence::LastChanged(bits) if STATE => {
                    count = changed::<STATE, PREFIX, PACKED, FLOAT_BYTES>(group.fields, from, to);
                    writer.write_bits(count as u32, bits)?;
                }
                Presence::Changed {
                    aggregate: true, ..
                } => {
                    let changed = group.fields.iter().any(|f| {
                        !f.equal::<STATE, PREFIX, PACKED, FLOAT_BYTES>(from[f.word], to[f.word])
                    });
                    writer.write_bits(u32::from(changed), 1)?;
                    if !changed {
                        continue;
                    }
                }
                _ => {}
            }
        }
        for field in &group.fields[..count] {
            let old = from[field.word];
            let new = to[field.word];
            if masked && field.flag != 0 && mask & field.flag == 0 {
                continue;
            }
            let bits = field.width::<PACKED>(mask);
            if compare {
                let changed = !field.equal::<STATE, PREFIX, PACKED, FLOAT_BYTES>(old, new);
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
                    writer.write_bits(new, bits)?;
                }
            } else {
                writer.write_bits(
                    field.project::<STATE, PREFIX, FLOAT_BYTES>(new) ^ field_key,
                    bits,
                )?;
            }
        }
    }
    Ok(())
}
#[inline(always)]
pub(crate) fn read<
    const STATE: bool,
    const PREFIX: bool,
    const PACKED: bool,
    const FLOAT_BYTES: bool,
>(
    groups: &[Group<STATE, PREFIX, PACKED, FLOAT_BYTES>],
    words: &mut [u32],
    key: u32,
    reader: &mut Reader<'_>,
) -> Result<(), Error> {
    for group in groups {
        let masked = matches!(
            group.presence,
            Presence::Mask(_) | Presence::OptionalMask(_) | Presence::MaskPreset { .. }
        ) || PREFIX;
        let compare = matches!(group.presence, Presence::Changed { .. })
            || STATE && matches!(group.presence, Presence::LastChanged(_));
        let (keyed, key_extra) = match group.presence {
            Presence::Changed {
                keyed, key_extra, ..
            } => (keyed, key_extra),
            _ => (false, 0),
        };
        let mut count = group.fields.len();
        let mask = if PREFIX {
            key
        } else {
            match group.presence {
                Presence::Mask(bits) => reader.read_bits(bits)?,
                Presence::MaskPreset { bits, .. } if STATE => reader.read_bits(bits)?,
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
            }
        };
        for field in &group.fields[..count] {
            let mut field_key = 0;
            if masked && field.flag != 0 && mask & field.flag == 0 {
                continue;
            }
            let bits = field.width::<PACKED>(mask);
            if compare && reader.read_bits(1)? == 0 {
                continue;
            }
            if keyed {
                field_key = key & (u32::MAX >> (32 - (bits + key_extra).min(32)));
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
                    reader.read_bits(bits)?
                }
            } else {
                field.restore::<STATE, PREFIX, PACKED, FLOAT_BYTES>(
                    reader.read_bits(bits)? ^ field_key,
                    bits,
                )
            };
        }
    }
    Ok(())
}
