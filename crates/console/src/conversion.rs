//! Pure alias projections from the C port's cvars_conversion.c. Calls occur at
//! command/module boundaries; engine frame consumers read cached canonical values.
use crate::{
    catalog::{Binding, Conversion, ConversionKind, Direction, Operation},
    cvars_generated::{MAPS, OPERANDS},
    numbers::number,
    views::{Context, Role, Source},
};
use std::fmt::{self, Write};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Error {
    NonFinite,
    TextTooLong,
    PolicyRequired,
    VideoPolicyRequired,
    InvalidBitWord,
}
#[derive(Clone, Debug)]
pub struct NumberText {
    bytes: [u8; 64],
    len: usize,
}
impl Write for NumberText {
    fn write_str(&mut self, value: &str) -> fmt::Result {
        let end = self.len.checked_add(value.len()).ok_or(fmt::Error)?;
        if end > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[self.len..end].copy_from_slice(value.as_bytes());
        self.len = end;
        Ok(())
    }
}
impl NumberText {
    fn new(value: f64) -> Result<Self, Error> {
        if !value.is_finite() {
            return Err(Error::NonFinite);
        }
        let mut text = Self {
            bytes: [0; 64],
            len: 0,
        };
        write!(text, "{value:.6}").map_err(|_| Error::TextTooLong)?;
        Ok(text)
    }
    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len]).unwrap_or("")
    }
}
pub enum Text<'a> {
    Borrowed(&'a str),
    Number(NumberText),
    Owned(String),
}
impl Text<'_> {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Borrowed(s) => s,
            Self::Number(n) => n.as_str(),
            Self::Owned(s) => s,
        }
    }
    pub fn into_owned(self) -> String {
        match self {
            Self::Borrowed(s) => s.to_owned(),
            Self::Number(n) => n.as_str().to_owned(),
            Self::Owned(s) => s,
        }
    }
    fn numeric(n: f64) -> Result<Self, Error> {
        Ok(Self::Number(NumberText::new(n)?))
    }
}
pub struct Input<'a, 'o> {
    pub context: Context,
    pub conversion: &'static Conversion,
    pub binding: &'static Binding,
    pub value: &'a str,
    pub current: &'a str,
    pub detail: Option<&'a str>,
    pub operand: &'o dyn Fn(u16) -> &'a str,
}
pub struct Change {
    pub row: u16,
    pub text: NumberText,
}
pub struct Output<'a> {
    pub text: Text<'a>,
    pub detail: bool,
    pub detail_value: Option<&'a str>,
    pub changes: [Option<Change>; 32],
    pub change_count: usize,
}
impl<'a> Output<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            text: Text::Borrowed(text),
            detail: false,
            detail_value: None,
            changes: std::array::from_fn(|_| None),
            change_count: 0,
        }
    }
    fn change(&mut self, row: u16, value: f64) -> Result<(), Error> {
        let target = self
            .changes
            .get_mut(self.change_count)
            .ok_or(Error::PolicyRequired)?;
        *target = Some(Change {
            row,
            text: NumberText::new(value)?,
        });
        self.change_count += 1;
        Ok(())
    }
}
fn truth(value: bool) -> f64 {
    if value { 1.0 } else { 0.0 }
}
fn competitive(value: f64, team: bool) -> bool {
    if team {
        (3.0..=7.0).contains(&value)
    } else {
        (0.0..=7.0).contains(&value) && value != 2.0
    }
}
fn mapped(c: &Conversion, value: f64, direction: Direction) -> f64 {
    MAPS[c.maps.clone()]
        .iter()
        .find(|m| {
            m.direction == direction
                && (if direction == Direction::AliasToCanonical {
                    m.alias
                } else {
                    m.canonical
                }) == value
        })
        .map_or(value, |m| {
            if direction == Direction::AliasToCanonical {
                m.canonical
            } else {
                m.alias
            }
        })
}
fn second_color(c: &Conversion) -> Result<u16, Error> {
    OPERANDS[c.operands.clone()]
        .last()
        .map(|o| o.row)
        .ok_or(Error::PolicyRequired)
}
fn numeric(in_: &Input<'_, '_>, text: &str) -> f64 {
    f64::from(number(text, in_.context.source))
}
fn bit(c: &crate::catalog::Operand, context: Context) -> u32 {
    if context.role == Role::Engine {
        c.unified_bit
    } else {
        c.source_bits[context.source as usize]
    }
}
pub fn read<'a>(in_: Input<'a, '_>) -> Result<Text<'a>, Error> {
    let c = in_.conversion;
    if c.cgame_only && in_.context.role != Role::Cgame {
        return Ok(Text::Borrowed(in_.value));
    }
    let mut value = numeric(&in_, in_.value);
    if c.detail
        && in_.detail.is_some()
        && (c.operation != Operation::Deathmatch || competitive(value, false))
        && (c.operation != Operation::Teamplay || competitive(value, true))
    {
        return Ok(Text::Borrowed(in_.detail.unwrap_or("")));
    }
    match c.operation {
        Operation::KhzHz => {
            value = match value {
                11.0 => 11025.0,
                22.0 => 22050.0,
                44.0 => 44100.0,
                48.0 => 48000.0,
                _ => value * 1000.0,
            }
        }
        #[allow(
            clippy::manual_clamp,
            reason = "Native fmin/fmax maps NaN to zero; clamp preserves NaN"
        )]
        Operation::Skill => value = (value - 1.0).max(0.0).min(3.0),
        Operation::ViewSize => {
            if !matches!(in_.context.source, Source::Quake | Source::QuakeWorld) {
                value = value.min(100.0);
            }
        }
        Operation::Deathmatch => value = truth(competitive(value, false)),
        Operation::Coop => value = truth(value == 9.0),
        Operation::Teamplay => value = truth(competitive(value, true)),
        Operation::Ctf => value = truth(value == 4.0),
        Operation::Autoswitch => {
            if in_
                .binding
                .name
                .eq_ignore_ascii_case("qts_weapon_autoswitch")
            {
                return Ok(Text::Borrowed(if value == 0.0 {
                    "never"
                } else {
                    "always"
                }));
            }
            value = if value == 0.0 { 3.0 } else { 1.0 };
        }
        Operation::InputGrab | Operation::OldRail | Operation::NoExit => {
            value = truth(value == 0.0)
        }
        Operation::NoSkins => value = truth(value != 0.0),
        Operation::Download => {
            if in_.context.source == Source::Quake2Rerelease && value < 0.0 {
                value = 0.0;
            }
        }
        Operation::MusicMute => {
            if cfg!(target_os = "linux") && in_.context.source == Source::Quake2 {
                value = truth(value != 0.0);
            }
        }
        Operation::ForceRespawn => value = truth(value > 0.0),
        Operation::Needpass => {
            if in_.context.source == Source::Quake3 {
                value = truth(value != 0.0);
            }
        }
        Operation::Sex => {
            return Ok(Text::Borrowed(
                if in_.value.eq_ignore_ascii_case("neuter") {
                    "none"
                } else {
                    in_.value
                },
            ));
        }
        Operation::QwSkin => {
            return Ok(Text::Borrowed(
                if in_.context.source == Source::QuakeWorld {
                    in_.value.rsplit('/').next().unwrap_or(in_.value)
                } else {
                    in_.value
                },
            ));
        }
        Operation::Color => value = mapped(c, value, Direction::CanonicalToAlias),
        Operation::PlayerColors => {
            value = mapped(c, value, Direction::CanonicalToAlias) * 16.0
                + mapped(
                    c,
                    numeric(&in_, (in_.operand)(second_color(c)?)),
                    Direction::CanonicalToAlias,
                )
        }
        Operation::Spectator => {
            if in_.context.source == Source::QuakeWorld {
                return Ok(Text::Borrowed(in_.value));
            }
            value = truth(!in_.value.is_empty() && in_.value != "0");
        }
        Operation::ClearColor => return Ok(Text::Borrowed(in_.value)),
        Operation::Fullscreen if value == 0.0 => value = 0.0,
        Operation::Fullscreen | Operation::VideoMode => return Err(Error::VideoPolicyRequired),
        Operation::None => match c.kind {
            ConversionKind::Composite if !c.operands.is_empty() => {
                let mut bits = 0u32;
                for op in &OPERANDS[c.operands.clone()] {
                    let n = numeric(&in_, (in_.operand)(op.row));
                    let set = if op.positive { n > 0.0 } else { n != 0.0 };
                    if set != op.inverted {
                        bits |= bit(op, in_.context);
                    }
                }
                value = f64::from(bits);
            }
            ConversionKind::Identity | ConversionKind::SideScope | ConversionKind::BitView => {
                return Ok(Text::Borrowed(in_.value));
            }
            ConversionKind::Reciprocal => value = 1.0 / value,
            ConversionKind::BoolInvert => value = truth(value == 0.0),
            ConversionKind::Linear | ConversionKind::ConsumerUnits => {
                value = value * c.scale + c.offset
            }
            _ => return Err(Error::PolicyRequired),
        },
        _ => match c.kind {
            ConversionKind::BoolInvert => value = truth(value == 0.0),
            ConversionKind::Linear => value = value * c.scale + c.offset,
            _ => {}
        },
    }
    Text::numeric(value)
}
pub fn write<'a>(in_: Input<'a, '_>) -> Result<Output<'a>, Error> {
    let c = in_.conversion;
    let mut out = Output::new(in_.value);
    if c.cgame_only && in_.context.role != Role::Cgame {
        return Ok(out);
    }
    let mut value = numeric(&in_, in_.value);
    let current = numeric(&in_, in_.current);
    out.detail = c.detail;
    match c.operation {
        Operation::KhzHz => {
            value = match value {
                11025.0 => 11.0,
                22050.0 => 22.0,
                44100.0 => 44.0,
                48000.0 => 48.0,
                _ => (value / 1000.0).round(),
            }
        }
        Operation::Skill => value += 1.0,
        Operation::ViewSize | Operation::ClearColor | Operation::Spectator => return Ok(out),
        Operation::Deathmatch => {
            value = if value > 0.0 {
                if competitive(current, true) {
                    current
                } else {
                    0.0
                }
            } else {
                8.0
            }
        }
        Operation::Coop => {
            value = if value != 0.0 {
                9.0
            } else if current == 9.0 {
                8.0
            } else {
                current
            }
        }
        Operation::Teamplay => {
            if in_.context.source == Source::Quake
                && (value == 1.0 || value == 2.0)
                && !c.operands.is_empty()
            {
                out.change(OPERANDS[c.operands.start].row, truth(value == 2.0))?;
            }
            value = if value > 0.0 {
                3.0
            } else if competitive(current, true) {
                0.0
            } else {
                current
            };
        }
        Operation::Ctf => {
            if value == 0.0 {
                return Err(Error::PolicyRequired);
            }
            value = 4.0;
        }
        Operation::ForceRespawn => value = if value != 0.0 { current.max(1.0) } else { 0.0 },
        Operation::Autoswitch => {
            value = if in_
                .binding
                .name
                .eq_ignore_ascii_case("qts_weapon_autoswitch")
            {
                truth(!in_.value.eq_ignore_ascii_case("never"))
            } else {
                truth(value != 3.0)
            }
        }
        Operation::Shadows => {
            value = if value == 0.0 {
                0.0
            } else if (2.0..=3.0).contains(&current) {
                current
            } else {
                1.0
            }
        }
        Operation::OldRail | Operation::InputGrab | Operation::NoExit => {
            value = truth(value == 0.0)
        }
        Operation::Gun
        | Operation::Footsteps
        | Operation::Lagometer
        | Operation::Draw2D
        | Operation::SoundBackend
        | Operation::BoolDetail
        | Operation::SameLevel => value = truth(value != 0.0),
        Operation::NoSkins => {
            value = truth(if in_.context.source == Source::QuakeWorld {
                value == 1.0
            } else {
                value != 0.0
            })
        }
        Operation::Download => {
            if in_.context.source == Source::Quake2Rerelease && value < 0.0 {
                value = 0.0;
            }
        }
        Operation::Sex => {
            out.text = Text::Borrowed(if in_.value.eq_ignore_ascii_case("none") {
                "neuter"
            } else {
                in_.value
            });
            return Ok(out);
        }
        Operation::Color => value = mapped(c, value, Direction::AliasToCanonical),
        Operation::PlayerColors => {
            let color = crate::numbers::integer(in_.value);
            out.change(
                second_color(c)?,
                mapped(c, f64::from(color & 15), Direction::AliasToCanonical),
            )?;
            value = mapped(c, f64::from((color >> 4) & 15), Direction::AliasToCanonical);
        }
        Operation::QwSkin => {
            if in_.context.source == Source::QuakeWorld {
                let prefix = in_.current.rfind('/').map_or("", |i| &in_.current[..=i]);
                let mut text = String::with_capacity(prefix.len() + in_.value.len());
                text.push_str(prefix);
                text.push_str(in_.value);
                out.text = Text::Owned(text);
            }
            return Ok(out);
        }
        Operation::MusicMute => {
            if cfg!(target_os = "linux") && in_.context.source == Source::Quake2 {
                if value == 0.0 {
                    if current != 0.0 {
                        out.detail = true;
                        out.detail_value = Some(in_.current);
                    }
                    value = 0.0;
                } else {
                    out.text = Text::Borrowed(if current != 0.0 {
                        in_.current
                    } else {
                        in_.detail.unwrap_or("1")
                    });
                    return Ok(out);
                }
            }
        }
        Operation::Fullscreen if value == 0.0 => value = 0.0,
        Operation::Fullscreen | Operation::VideoMode => return Err(Error::VideoPolicyRequired),
        _ => match c.kind {
            ConversionKind::Composite if !c.operands.is_empty() => {
                if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(u32::MAX)
                {
                    return Err(Error::InvalidBitWord);
                }
                let bits = if value < 0.0 {
                    value as i32 as u32
                } else {
                    value as u32
                };
                for op in &OPERANDS[c.operands.clone()] {
                    let mask = bit(op, in_.context);
                    if mask == 0 {
                        continue;
                    }
                    let set = ((bits & mask) != 0) != op.inverted;
                    let mut n = truth(set);
                    if set && op.positive {
                        n = n.max(numeric(&in_, (in_.operand)(op.row)));
                    }
                    out.change(op.row, n)?;
                }
                return Ok(out);
            }
            ConversionKind::Identity
            | ConversionKind::SideScope
            | ConversionKind::EnumDetail
            | ConversionKind::BitView => return Ok(out),
            ConversionKind::Reciprocal => value = 1.0 / value,
            ConversionKind::BoolInvert => value = truth(value == 0.0),
            ConversionKind::Linear | ConversionKind::ConsumerUnits => {
                value = (value - c.offset) / c.scale
            }
            _ => return Err(Error::PolicyRequired),
        },
    }
    out.text = Text::numeric(value)?;
    Ok(out)
}
