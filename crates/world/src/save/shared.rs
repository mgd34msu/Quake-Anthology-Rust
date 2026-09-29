//! Shared save readers ported from `src/persistence/shared.ts`.
//!
//! Time, vectors, bounds, frames, clocks, arithmetic/numeric profiles,
//! random states, and frame orderings. Wire keys match the donor exactly;
//! values reuse [`qa_core`] time/math/numeric types where the shapes agree.
//! [`SavedNumericProfile`] keeps a owned namespaced id (the core profile
//! id is `&'static str`), and [`SaveRandomState`] covers the donor random
//! sources the core `Qrand` does not model.

use qa_core::identity::ProviderId;
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::{Arithmetic, DonorSource, FloatToInt, Rounding, X87Precision};
use qa_core::time::{ClockProfile, FrameContext, FramePhase, SourceTime};

use super::value::{arr, boolean, int, namespaced, num, obj, save_error, str, SaveJson, SaveReader};
use crate::scheduler::FrameOrdering;
use crate::WorldError;

/// Validate a `sha256:` digest with 64 lowercase hex digits.
pub fn validate_digest(value: &str) -> Result<(), WorldError> {
    if value.len() == 71
        && value.starts_with("sha256:")
        && value[7..]
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
    {
        Ok(())
    } else {
        Err(save_error("digest", "expected a SHA-256 content digest"))
    }
}

/// Validate a `family:edition:package:revision` content id.
pub fn validate_content_id(value: &str) -> Result<(), WorldError> {
    let mut parts = value.split(':');
    let family = parts.next().unwrap_or("");
    if !matches!(family, "q1" | "q2" | "q3") {
        return Err(save_error("content", "expected a content identity"));
    }
    for _ in 0..3 {
        match parts.next() {
            Some(part) if valid_identity_part(part) => {}
            _ => return Err(save_error("content", "expected a content identity")),
        }
    }
    if parts.next().is_some() {
        return Err(save_error("content", "expected a content identity"));
    }
    Ok(())
}

fn valid_identity_part(part: &str) -> bool {
    let mut chars = part.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphanumeric() => {}
        _ => return false,
    }
    chars.all(|character| character.is_ascii_alphanumeric() || matches!(character, '.' | '_' | '+' | '-'))
}

/// Validate a `prefix:a:b` save identity and return its parts.
pub fn identity_parts(reader: SaveReader, prefix: &str) -> Result<(String, String), WorldError> {
    let value = reader.string()?;
    let mut parts = value.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some(head), Some(a), Some(b), None) if head == prefix && !a.is_empty() && !b.is_empty() => {
            if valid_identity_part(a) && valid_identity_part(b) {
                Ok((a.to_string(), b.to_string()))
            } else {
                Err(reader.fail(&format!("expected a {prefix} identity")))
            }
        }
        _ => Err(reader.fail(&format!("expected a {prefix} identity"))),
    }
}

/// Read a content digest.
pub fn read_digest(reader: SaveReader) -> Result<String, WorldError> {
    let value = reader.string()?;
    validate_digest(&value).map_err(|_| reader.fail("expected a SHA-256 content digest"))?;
    Ok(value)
}

/// Read a content id.
pub fn read_content_id(reader: SaveReader) -> Result<String, WorldError> {
    let value = reader.string()?;
    validate_content_id(&value).map_err(|_| reader.fail("expected a content identity"))?;
    Ok(value)
}

/// Read source time (`seconds` f32 / `milliseconds` i32).
pub fn read_time(reader: SaveReader) -> Result<SourceTime, WorldError> {
    let kind = reader.field("kind").choice_str(&["seconds", "milliseconds"])?;
    let value = reader.field("value").finite()?;
    if kind == "seconds" {
        #[allow(clippy::cast_possible_truncation)]
        Ok(SourceTime::Seconds(value as f32))
    } else {
        if !(f64::from(i32::MIN)..=f64::from(i32::MAX)).contains(&value) {
            return Err(reader.field("value").fail("expected a finite number"));
        }
        #[allow(clippy::cast_possible_truncation)]
        Ok(SourceTime::Milliseconds(value.trunc() as i32))
    }
}

/// Write source time.
#[must_use]
pub fn write_time(time: SourceTime) -> SaveJson {
    match time {
        SourceTime::Seconds(value) => obj(vec![("kind", str("seconds")), ("value", num(f64::from(value)))]),
        SourceTime::Milliseconds(value) => obj(vec![("kind", str("milliseconds")), ("value", num(f64::from(value)))]),
    }
}

/// Read a vector (donor `number()` accepts non-finite components).
pub fn read_vector(reader: SaveReader) -> Result<Vec3, WorldError> {
    #[allow(clippy::cast_possible_truncation)]
    Ok(Vec3 {
        x: reader.field("x").number()? as f32,
        y: reader.field("y").number()? as f32,
        z: reader.field("z").number()? as f32,
    })
}

/// Write a vector.
#[must_use]
pub fn write_vector(value: Vec3) -> SaveJson {
    obj(vec![
        ("x", num(f64::from(value.x))),
        ("y", num(f64::from(value.y))),
        ("z", num(f64::from(value.z))),
    ])
}

/// Read bounds.
pub fn read_bounds(reader: SaveReader) -> Result<Bounds, WorldError> {
    Ok(Bounds {
        min: read_vector(reader.field("min"))?,
        max: read_vector(reader.field("max"))?,
    })
}

/// Write bounds.
#[must_use]
pub fn write_bounds(value: Bounds) -> SaveJson {
    obj(vec![("min", write_vector(value.min)), ("max", write_vector(value.max))])
}

const PHASES: &[(&str, FramePhase)] = &[
    ("frame-entry", FramePhase::FrameEntry),
    ("client-command", FramePhase::ClientCommand),
    ("entity-prethink", FramePhase::EntityPrethink),
    ("entity-physics", FramePhase::EntityPhysics),
    ("entity-think", FramePhase::EntityThink),
    ("client-end-frame", FramePhase::ClientEndFrame),
    ("frame-exit", FramePhase::FrameExit),
];

/// Read a frame context.
pub fn read_frame(reader: SaveReader) -> Result<FrameContext, WorldError> {
    let frame = reader.field("frame").integer(0)?;
    let frame = i32::try_from(frame).map_err(|_| reader.field("frame").fail("expected an integer in range"))?;
    let phase_name = reader.field("phase").choice_str(&[
        "frame-entry",
        "client-command",
        "entity-prethink",
        "entity-physics",
        "entity-think",
        "client-end-frame",
        "frame-exit",
    ])?;
    let phase = PHASES
        .iter()
        .find(|(name, _)| *name == phase_name)
        .map(|(_, phase)| *phase)
        .ok_or_else(|| reader.field("phase").fail("expected a frame phase"))?;
    Ok(FrameContext {
        frame,
        time: read_time(reader.field("time"))?,
        elapsed: read_time(reader.field("elapsed"))?,
        phase,
    })
}

/// Write a frame context.
#[must_use]
pub fn write_frame(frame: FrameContext) -> SaveJson {
    let phase = PHASES
        .iter()
        .find(|(_, candidate)| *candidate == frame.phase)
        .map_or("frame-entry", |(name, _)| *name);
    obj(vec![
        ("frame", int(i64::from(frame.frame))),
        ("time", write_time(frame.time)),
        ("elapsed", write_time(frame.elapsed)),
        ("phase", str(phase)),
    ])
}

/// Read a clock profile.
pub fn read_clock(reader: SaveReader) -> Result<ClockProfile, WorldError> {
    let kind =
        reader
            .field("kind")
            .choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?;
    match kind.as_str() {
        "q1-netquake" => Ok(ClockProfile::Q1Netquake {
            minimum_frame_seconds: reader.field("minimumFrameSeconds").finite()?,
            maximum_frame_seconds: reader.field("maximumFrameSeconds").finite()?,
            fixed_frame_seconds: reader.field("fixedFrameSeconds").nullable(|value| value.finite())?,
        }),
        "q1-quakeworld" => Ok(ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds: reader.field("maximumCommandMilliseconds").finite()?,
        }),
        "q2-classic" => {
            reader.field("frameMilliseconds").literal_i64(100)?;
            Ok(ClockProfile::Q2Classic)
        }
        "q2-rerelease" => {
            let frame_milliseconds = reader.field("frameMilliseconds").finite()?;
            reader.field("preparation").literal_str("before-frame")?;
            Ok(ClockProfile::Q2Rerelease { frame_milliseconds })
        }
        _ => Ok(ClockProfile::Q3 {
            server_frame_milliseconds: reader.field("serverFrameMilliseconds").finite()?,
            fixed_movement_milliseconds: reader
                .field("fixedMovementMilliseconds")
                .nullable(|value| value.finite())?,
        }),
    }
    .and_then(|profile| {
        if kind == "q3" {
            reader.field("maximumCommandMilliseconds").literal_i64(200)?;
        }
        Ok(profile)
    })
}

/// Write a clock profile.
#[must_use]
pub fn write_clock(clock: ClockProfile) -> SaveJson {
    match clock {
        ClockProfile::Q1Netquake {
            minimum_frame_seconds,
            maximum_frame_seconds,
            fixed_frame_seconds,
        } => obj(vec![
            ("kind", str("q1-netquake")),
            ("minimumFrameSeconds", num(minimum_frame_seconds)),
            ("maximumFrameSeconds", num(maximum_frame_seconds)),
            ("fixedFrameSeconds", fixed_frame_seconds.map_or(SaveJson::Null, num)),
        ]),
        ClockProfile::Q1Quakeworld {
            maximum_command_milliseconds,
        } => obj(vec![
            ("kind", str("q1-quakeworld")),
            ("maximumCommandMilliseconds", num(maximum_command_milliseconds)),
        ]),
        ClockProfile::Q2Classic => obj(vec![("kind", str("q2-classic")), ("frameMilliseconds", int(100))]),
        ClockProfile::Q2Rerelease { frame_milliseconds } => obj(vec![
            ("kind", str("q2-rerelease")),
            ("frameMilliseconds", num(frame_milliseconds)),
            ("preparation", str("before-frame")),
        ]),
        ClockProfile::Q3 {
            server_frame_milliseconds,
            fixed_movement_milliseconds,
        } => obj(vec![
            ("kind", str("q3")),
            ("serverFrameMilliseconds", num(server_frame_milliseconds)),
            (
                "fixedMovementMilliseconds",
                fixed_movement_milliseconds.map_or(SaveJson::Null, num),
            ),
            ("maximumCommandMilliseconds", int(200)),
        ]),
    }
}

fn read_rounding(reader: SaveReader) -> Result<Rounding, WorldError> {
    let name = reader.choice_str(&["nearest-even", "toward-zero", "toward-positive", "toward-negative"])?;
    match name.as_str() {
        "nearest-even" => Ok(Rounding::NearestEven),
        "toward-zero" => Ok(Rounding::TowardZero),
        "toward-positive" => Ok(Rounding::TowardPositive),
        _ => Ok(Rounding::TowardNegative),
    }
}

fn write_rounding(rounding: Rounding) -> SaveJson {
    str(match rounding {
        Rounding::NearestEven => "nearest-even",
        Rounding::TowardZero => "toward-zero",
        Rounding::TowardPositive => "toward-positive",
        Rounding::TowardNegative => "toward-negative",
    })
}

/// Read an arithmetic profile.
pub fn read_arithmetic(reader: SaveReader) -> Result<Arithmetic, WorldError> {
    let kind = reader
        .field("kind")
        .choice_str(&["binary32", "donor-binary64", "x87", "sse"])?;
    match kind.as_str() {
        "binary32" => {
            reader.field("round").literal_str("each-operation")?;
            Ok(Arithmetic::Binary32EachOp)
        }
        "donor-binary64" => {
            let source = reader.field("source").choice_str(&["q1-ts", "q2-ts"])?;
            Ok(Arithmetic::DonorBinary64(if source == "q1-ts" {
                DonorSource::Q1
            } else {
                DonorSource::Q2
            }))
        }
        "x87" => {
            let precision = reader.field("precisionBits").choice_i64(&[24, 53, 64])?;
            Ok(Arithmetic::X87 {
                precision_bits: match precision {
                    24 => X87Precision::Bits24,
                    53 => X87Precision::Bits53,
                    _ => X87Precision::Bits64,
                },
                rounding: read_rounding(reader.field("rounding"))?,
            })
        }
        _ => Ok(Arithmetic::Sse {
            flush_to_zero: reader.field("flushToZero").boolean()?,
            denormals_are_zero: reader.field("denormalsAreZero").boolean()?,
            rounding: read_rounding(reader.field("rounding"))?,
        }),
    }
}

/// Write an arithmetic profile.
#[must_use]
pub fn write_arithmetic(arithmetic: Arithmetic) -> SaveJson {
    match arithmetic {
        Arithmetic::Binary32EachOp => obj(vec![("kind", str("binary32")), ("round", str("each-operation"))]),
        Arithmetic::DonorBinary64(source) => obj(vec![
            ("kind", str("donor-binary64")),
            (
                "source",
                str(match source {
                    DonorSource::Q1 => "q1-ts",
                    DonorSource::Q2 => "q2-ts",
                }),
            ),
        ]),
        Arithmetic::X87 {
            precision_bits,
            rounding,
        } => obj(vec![
            ("kind", str("x87")),
            (
                "precisionBits",
                int(match precision_bits {
                    X87Precision::Bits24 => 24,
                    X87Precision::Bits53 => 53,
                    X87Precision::Bits64 => 64,
                }),
            ),
            ("rounding", write_rounding(rounding)),
        ]),
        Arithmetic::Sse {
            flush_to_zero,
            denormals_are_zero,
            rounding,
        } => obj(vec![
            ("kind", str("sse")),
            ("flushToZero", boolean(flush_to_zero)),
            ("denormalsAreZero", boolean(denormals_are_zero)),
            ("rounding", write_rounding(rounding)),
        ]),
    }
}

/// Saved numeric profile (owned namespaced id; storage is always binary32).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SavedNumericProfile {
    /// Profile id (`namespace:name`).
    pub id: String,
    /// Operation rounding.
    pub arithmetic: Arithmetic,
    /// Float-to-int rule.
    pub float_to_int: FloatToInt,
}

/// Read a numeric profile.
pub fn read_numeric(reader: SaveReader) -> Result<SavedNumericProfile, WorldError> {
    let float_name =
        reader
            .field("floatToInt")
            .choice_str(&["qvm-indefinite", "x86-indefinite", "checked-c-truncation"])?;
    reader.field("scalarStorage").literal_str("binary32")?;
    reader.field("integerOverflow").literal_str("wrap32")?;
    Ok(SavedNumericProfile {
        id: namespaced(reader.field("id"))?,
        arithmetic: read_arithmetic(reader.field("arithmetic"))?,
        float_to_int: match float_name.as_str() {
            "qvm-indefinite" => FloatToInt::QvmIndefinite,
            "x86-indefinite" => FloatToInt::X86Indefinite,
            _ => FloatToInt::CheckedTruncation,
        },
    })
}

/// Write a numeric profile.
#[must_use]
pub fn write_numeric(profile: &SavedNumericProfile) -> SaveJson {
    obj(vec![
        ("id", str(&profile.id)),
        ("arithmetic", write_arithmetic(profile.arithmetic)),
        ("scalarStorage", str("binary32")),
        (
            "floatToInt",
            str(match profile.float_to_int {
                FloatToInt::QvmIndefinite => "qvm-indefinite",
                FloatToInt::X86Indefinite => "x86-indefinite",
                FloatToInt::CheckedTruncation => "checked-c-truncation",
            }),
        ),
        ("integerOverflow", str("wrap32")),
    ])
}

/// Saved random state covering every donor source.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveRandomState {
    /// Q3 linear congruential generator.
    Q3Lcg {
        /// Seed.
        seed: i64,
        /// Draw count.
        draws: u64,
    },
    /// MSVCRT `rand` state.
    MsvcrtRand {
        /// Seed.
        seed: i64,
        /// Draw count.
        draws: u64,
    },
    /// glibc `random` state.
    GlibcRandom {
        /// State words.
        words: Vec<i64>,
        /// Front index.
        front: i64,
        /// Rear index.
        rear: i64,
        /// Draw count.
        draws: u64,
    },
    /// Q2 rerelease MT19937 state.
    RereleaseMt19937 {
        /// State words (624).
        words: Vec<u32>,
        /// Current index (0 through 624).
        index: u32,
        /// Draw count.
        draws: u64,
    },
    /// Guest-owned random bytes.
    Guest {
        /// Owning module.
        module: String,
        /// State bytes.
        bytes: Vec<u8>,
        /// Draw count.
        draws: u64,
    },
}

fn read_draws(reader: SaveReader) -> Result<u64, WorldError> {
    let draws = reader.field("draws").integer(0)?;
    u64::try_from(draws).map_err(|_| reader.field("draws").fail("expected an integer in range"))
}

/// Read a random state.
pub fn read_random(reader: SaveReader) -> Result<SaveRandomState, WorldError> {
    let draws = read_draws(reader.clone())?;
    let kind =
        reader
            .field("kind")
            .choice_str(&["q3-lcg", "msvcrt-rand", "glibc-random", "q2-rerelease-mt19937", "guest"])?;
    match kind.as_str() {
        "q3-lcg" => Ok(SaveRandomState::Q3Lcg {
            seed: reader.field("seed").integer(i64::MIN)?,
            draws,
        }),
        "msvcrt-rand" => Ok(SaveRandomState::MsvcrtRand {
            seed: reader.field("seed").integer(i64::MIN)?,
            draws,
        }),
        "glibc-random" => Ok(SaveRandomState::GlibcRandom {
            words: reader.field("words").list(|item| item.integer(i64::MIN))?,
            front: reader.field("front").integer(0)?,
            rear: reader.field("rear").integer(0)?,
            draws,
        }),
        "q2-rerelease-mt19937" => {
            let words = reader.field("words").list(|item| {
                let word = item.integer(0)?;
                if word > i64::from(u32::MAX) {
                    return Err(item.fail("expected a uint32 MT19937 word"));
                }
                #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
                Ok(word as u32)
            })?;
            if words.len() != 624 {
                return Err(reader.field("words").fail("expected 624 MT19937 words"));
            }
            let index = reader.field("index").integer(0)?;
            if index > 624 {
                return Err(reader.field("index").fail("expected MT19937 index 0 through 624"));
            }
            reader.field("distribution").literal_str("msvc-2022-17.6")?;
            #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
            Ok(SaveRandomState::RereleaseMt19937 {
                words,
                index: index as u32,
                draws,
            })
        }
        _ => Ok(SaveRandomState::Guest {
            module: reader.field("module").string()?,
            bytes: reader.field("bytes").bytes()?,
            draws,
        }),
    }
}

/// Write a random state.
#[must_use]
pub fn write_random(state: &SaveRandomState) -> SaveJson {
    let draws = match state {
        SaveRandomState::Q3Lcg { draws, .. }
        | SaveRandomState::MsvcrtRand { draws, .. }
        | SaveRandomState::GlibcRandom { draws, .. }
        | SaveRandomState::RereleaseMt19937 { draws, .. }
        | SaveRandomState::Guest { draws, .. } => *draws,
    };
    #[allow(clippy::cast_possible_wrap)]
    let draws_json = int(draws as i64);
    match state {
        SaveRandomState::Q3Lcg { seed, .. } => obj(vec![
            ("kind", str("q3-lcg")),
            ("seed", int(*seed)),
            ("draws", draws_json),
        ]),
        SaveRandomState::MsvcrtRand { seed, .. } => obj(vec![
            ("kind", str("msvcrt-rand")),
            ("seed", int(*seed)),
            ("draws", draws_json),
        ]),
        SaveRandomState::GlibcRandom { words, front, rear, .. } => obj(vec![
            ("kind", str("glibc-random")),
            ("words", arr(words.iter().map(|word| int(*word)).collect())),
            ("front", int(*front)),
            ("rear", int(*rear)),
            ("draws", draws_json),
        ]),
        SaveRandomState::RereleaseMt19937 { words, index, .. } => obj(vec![
            ("kind", str("q2-rerelease-mt19937")),
            ("distribution", str("msvc-2022-17.6")),
            ("words", arr(words.iter().map(|word| int(i64::from(*word))).collect())),
            ("index", int(i64::from(*index))),
            ("draws", draws_json),
        ]),
        SaveRandomState::Guest { module, bytes, .. } => obj(vec![
            ("kind", str("guest")),
            ("module", str(module)),
            ("bytes", SaveJson::Bytes(bytes.clone())),
            ("draws", draws_json),
        ]),
    }
}

/// Read a frame ordering.
pub fn read_ordering(reader: SaveReader) -> Result<FrameOrdering, WorldError> {
    let kind = reader.field("kind").choice_str(&["native", "mixed"])?;
    if kind == "native" {
        reader.field("traversal").literal_str("source-slot-order")?;
        Ok(FrameOrdering::Native {
            clock: read_clock(reader.field("clock"))?,
        })
    } else {
        let providers = reader.field("providers").list(|item| {
            let name = namespaced(item)?;
            let (namespace, id) = name.split_once(':').expect("validated namespaced id");
            Ok(ProviderId::new(namespace, id))
        })?;
        reader.field("entityOrder").literal_str("source-slot-order")?;
        reader.field("ties").literal_str("provider-entity-invocation")?;
        Ok(FrameOrdering::Mixed { providers })
    }
}

/// Write a frame ordering.
#[must_use]
pub fn write_ordering(ordering: &FrameOrdering) -> SaveJson {
    match ordering {
        FrameOrdering::Native { clock } => obj(vec![
            ("kind", str("native")),
            ("traversal", str("source-slot-order")),
            ("clock", write_clock(*clock)),
        ]),
        FrameOrdering::Mixed { providers } => obj(vec![
            ("kind", str("mixed")),
            (
                "providers",
                arr(providers
                    .iter()
                    .map(|provider| str(&format!("{}:{}", provider.namespace, provider.name)))
                    .collect()),
            ),
            ("entityOrder", str("source-slot-order")),
            ("ties", str("provider-entity-invocation")),
        ]),
    }
}

/// Provider + content reference (donor `ProviderReference`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderRef {
    /// Provider (`namespace:name`).
    pub provider: String,
    /// Content id.
    pub content: String,
}

/// Read a provider reference.
pub fn read_provider_ref(reader: SaveReader) -> Result<ProviderRef, WorldError> {
    Ok(ProviderRef {
        provider: namespaced(reader.field("provider"))?,
        content: read_content_id(reader.field("content"))?,
    })
}

/// Write a provider reference.
#[must_use]
pub fn write_provider_ref(value: &ProviderRef) -> SaveJson {
    obj(vec![
        ("provider", str(&value.provider)),
        ("content", str(&value.content)),
    ])
}

/// Character definition + appearance selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterSelection {
    /// Definition reference.
    pub definition: ProviderRef,
    /// Appearance reference.
    pub appearance: ProviderRef,
}

/// Read a character selection.
pub fn read_character(reader: SaveReader) -> Result<CharacterSelection, WorldError> {
    Ok(CharacterSelection {
        definition: read_provider_ref(reader.field("definition"))?,
        appearance: read_provider_ref(reader.field("appearance"))?,
    })
}

/// Write a character selection.
#[must_use]
pub fn write_character(value: &CharacterSelection) -> SaveJson {
    obj(vec![
        ("definition", write_provider_ref(&value.definition)),
        ("appearance", write_provider_ref(&value.appearance)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::save::value::{decode_checkpoint_value, encode_checkpoint_value, SaveReader};

    fn round_trip(value: &SaveJson) -> SaveJson {
        decode_checkpoint_value(&encode_checkpoint_value(value)).unwrap()
    }

    #[test]
    fn time_vector_frame_clocks_round_trip() {
        let vectors = [
            (
                Vec3 {
                    x: 1.5,
                    y: -2.0,
                    z: 0.0,
                },
                "plain",
            ),
            (
                Vec3 {
                    x: f32::NAN,
                    y: f32::INFINITY,
                    z: -0.0,
                },
                "tagged",
            ),
        ];
        for (vector, _) in vectors {
            let json = write_vector(vector);
            let back = read_vector(SaveReader::at(&round_trip(&json), "v")).unwrap();
            assert_eq!(back.x.to_bits(), vector.x.to_bits());
            assert_eq!(back.y.to_bits(), vector.y.to_bits());
            assert_eq!(back.z.to_bits(), vector.z.to_bits());
        }
        for time in [SourceTime::Seconds(1.25), SourceTime::Milliseconds(-42)] {
            let json = write_time(time);
            assert_eq!(read_time(SaveReader::at(&round_trip(&json), "t")).unwrap(), time);
        }
        let frame = FrameContext {
            frame: 7,
            time: SourceTime::Seconds(1.0),
            elapsed: SourceTime::Milliseconds(16),
            phase: FramePhase::EntityThink,
        };
        let json = write_frame(frame);
        assert_eq!(read_frame(SaveReader::at(&round_trip(&json), "f")).unwrap(), frame);
        for clock in [
            ClockProfile::Q1Netquake {
                minimum_frame_seconds: 0.001,
                maximum_frame_seconds: 0.1,
                fixed_frame_seconds: None,
            },
            ClockProfile::Q1Quakeworld {
                maximum_command_milliseconds: 50.0,
            },
            ClockProfile::Q2Classic,
            ClockProfile::Q2Rerelease {
                frame_milliseconds: 16.66,
            },
            ClockProfile::Q3 {
                server_frame_milliseconds: 8.0,
                fixed_movement_milliseconds: Some(8.0),
            },
        ] {
            let json = write_clock(clock);
            assert_eq!(read_clock(SaveReader::at(&round_trip(&json), "c")).unwrap(), clock);
        }
        let bad = obj(vec![("kind", str("q2-classic")), ("frameMilliseconds", int(99))]);
        assert!(read_clock(SaveReader::new(&bad)).is_err());
    }

    #[test]
    fn numeric_random_ordering_round_trip() {
        let profile = SavedNumericProfile {
            id: "q3:binary32".to_string(),
            arithmetic: Arithmetic::Sse {
                flush_to_zero: true,
                denormals_are_zero: false,
                rounding: Rounding::TowardZero,
            },
            float_to_int: FloatToInt::CheckedTruncation,
        };
        let json = write_numeric(&profile);
        assert_eq!(read_numeric(SaveReader::at(&round_trip(&json), "n")).unwrap(), profile);
        let states = vec![
            SaveRandomState::Q3Lcg { seed: -5, draws: 12 },
            SaveRandomState::MsvcrtRand { seed: 1, draws: 0 },
            SaveRandomState::GlibcRandom {
                words: vec![1, 2, 3],
                front: 0,
                rear: 1,
                draws: 4,
            },
            SaveRandomState::RereleaseMt19937 {
                words: vec![7u32; 624],
                index: 624,
                draws: 9,
            },
            SaveRandomState::Guest {
                module: "q3:game".to_string(),
                bytes: vec![1, 2, 3],
                draws: 2,
            },
        ];
        for state in &states {
            let json = write_random(state);
            assert_eq!(read_random(SaveReader::at(&round_trip(&json), "r")).unwrap(), *state);
        }
        let bad_words = obj(vec![
            ("kind", str("q2-rerelease-mt19937")),
            ("distribution", str("msvc-2022-17.6")),
            ("words", arr(vec![int(1)])),
            ("index", int(0)),
            ("draws", int(0)),
        ]);
        assert!(read_random(SaveReader::new(&bad_words)).is_err());
        for ordering in [
            FrameOrdering::Native {
                clock: ClockProfile::Q2Classic,
            },
            FrameOrdering::Mixed {
                providers: vec![ProviderId::new("q3", "game")],
            },
        ] {
            let json = write_ordering(&ordering);
            assert_eq!(
                read_ordering(SaveReader::at(&round_trip(&json), "o")).unwrap(),
                ordering
            );
        }
    }

    #[test]
    fn identities_validate() {
        assert!(validate_digest("sha256:0000000000000000000000000000000000000000000000000000000000000000").is_ok());
        assert!(validate_digest("sha256:XYZ").is_err());
        assert!(validate_digest("SHA256:0000000000000000000000000000000000000000000000000000000000000000").is_err());
        assert!(validate_content_id("q2:classic:base:1").is_ok());
        assert!(validate_content_id("q9:classic:base:1").is_err());
        assert!(validate_content_id("q2:classic:base").is_err());
    }
}
