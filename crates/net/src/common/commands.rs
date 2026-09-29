//! Unified actor command codec ported from `src/network/common/commands.ts`.
//!
//! Version 1 preserves numeric values exactly; the selected provider performs
//! its own source rounding. Used only after unified composition admission;
//! this packet is never a source-native move message.

use qa_core::identity::{ActorId, ClientId, IdentityOwner, ProviderId, SeatId};
use thiserror::Error;

/// Error for unified command coding failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum CommandError {
    /// Unified command identifier must have a namespace and name.
    #[error("Unified command identifier must have a namespace and name")]
    BadIdentifier,
    /// Unified command identifier is too long.
    #[error("Unified command identifier is too long")]
    IdentifierTooLong,
    /// Unified command contains a non-finite number.
    #[error("Unified command contains a non-finite number")]
    NonFiniteNumber,
    /// Unified command sequence must be a nonnegative safe integer.
    #[error("Unified command sequence must be a nonnegative safe integer")]
    BadSequence,
    /// Unknown unified movement command dialect.
    #[error("Unknown unified movement command dialect")]
    BadDialect,
    /// Invalid unified weapon selection tag.
    #[error("Invalid unified weapon selection tag")]
    BadWeaponTag,
    /// Invalid unified holdable state.
    #[error("Invalid unified holdable state")]
    BadHoldable,
    /// Invalid unified arsenal intent tag.
    #[error("Invalid unified arsenal intent tag")]
    BadIntentTag,
    /// Trailing unified command bytes.
    #[error("Trailing unified command bytes")]
    TrailingBytes,
    /// Unified command actor is stale or not controlled by this source.
    #[error("Unified command actor is stale or not controlled by this source")]
    StaleActor,
    /// Unified command source belongs to another session.
    #[error("Unified command source belongs to another session")]
    ForeignSource,
    /// Unified movement dialect differs from the selected actor provider.
    #[error("Unified movement dialect differs from the selected actor provider")]
    DialectMismatch,
    /// Unified arsenal intent belongs to another provider.
    #[error("Unified arsenal intent belongs to another provider")]
    ProviderMismatch,
    /// Unsupported unified command version.
    #[error("Unsupported unified command version")]
    BadVersion,
    /// Truncated unified command.
    #[error("Truncated unified command at byte {0}")]
    Truncated(usize),
    /// Bad unified command magic.
    #[error("Not a unified command")]
    BadMagic,
    /// Unified command text is not valid UTF-8.
    #[error("Invalid unified command text")]
    BadText,
}

/// Movement dialect tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovementDialect {
    /// NetQuake.
    Q1Netquake,
    /// QuakeWorld.
    Q1Quakeworld,
    /// Quake II classic.
    Q2Classic,
    /// Quake II rerelease.
    Q2Rerelease,
    /// Quake III.
    Q3,
}

/// Unified movement command (`UserCommand`).
#[derive(Debug, Clone, PartialEq)]
pub enum UserCommand {
    /// NetQuake move.
    Q1Netquake {
        /// Acknowledged server time in seconds.
        acknowledged_server_time_seconds: f64,
        /// View angles.
        view_angles: [f64; 3],
        /// Forward move.
        forward_move: f64,
        /// Side move.
        side_move: f64,
        /// Up move.
        up_move: f64,
        /// Buttons.
        buttons: f64,
        /// Impulse.
        impulse: f64,
    },
    /// QuakeWorld move.
    Q1Quakeworld {
        /// Milliseconds.
        milliseconds: f64,
        /// Angles.
        angles: [f64; 3],
        /// Forward move.
        forward_move: f64,
        /// Side move.
        side_move: f64,
        /// Up move.
        up_move: f64,
        /// Buttons.
        buttons: f64,
        /// Impulse.
        impulse: f64,
    },
    /// Quake II classic move.
    Q2Classic {
        /// Milliseconds.
        milliseconds: f64,
        /// Angle shorts.
        angle_shorts: [f64; 3],
        /// Forward move.
        forward_move: f64,
        /// Side move.
        side_move: f64,
        /// Up move.
        up_move: f64,
        /// Buttons.
        buttons: f64,
        /// Impulse.
        impulse: f64,
        /// Light level.
        light_level: f64,
    },
    /// Quake II rerelease move.
    Q2Rerelease {
        /// Milliseconds.
        milliseconds: f64,
        /// Angles.
        angles: [f64; 3],
        /// Forward move.
        forward_move: f64,
        /// Side move.
        side_move: f64,
        /// Buttons.
        buttons: f64,
        /// Server frame.
        server_frame: f64,
    },
    /// Quake III move.
    Q3 {
        /// Server time in milliseconds.
        server_time_milliseconds: f64,
        /// Angle words.
        angle_words: [f64; 3],
        /// Buttons.
        buttons: f64,
        /// Weapon.
        weapon: f64,
        /// Forward move.
        forward_move: f64,
        /// Right move.
        right_move: f64,
        /// Up move.
        up_move: f64,
    },
}

impl UserCommand {
    /// Movement dialect.
    #[must_use]
    pub fn dialect(&self) -> MovementDialect {
        match self {
            Self::Q1Netquake { .. } => MovementDialect::Q1Netquake,
            Self::Q1Quakeworld { .. } => MovementDialect::Q1Quakeworld,
            Self::Q2Classic { .. } => MovementDialect::Q2Classic,
            Self::Q2Rerelease { .. } => MovementDialect::Q2Rerelease,
            Self::Q3 { .. } => MovementDialect::Q3,
        }
    }

    fn fields(&self) -> Vec<f64> {
        match self {
            Self::Q1Netquake {
                acknowledged_server_time_seconds,
                view_angles,
                forward_move,
                side_move,
                up_move,
                buttons,
                impulse,
            } => vec![
                *acknowledged_server_time_seconds,
                view_angles[0],
                view_angles[1],
                view_angles[2],
                *forward_move,
                *side_move,
                *up_move,
                *buttons,
                *impulse,
            ],
            Self::Q1Quakeworld {
                milliseconds,
                angles,
                forward_move,
                side_move,
                up_move,
                buttons,
                impulse,
            } => vec![
                *milliseconds,
                angles[0],
                angles[1],
                angles[2],
                *forward_move,
                *side_move,
                *up_move,
                *buttons,
                *impulse,
            ],
            Self::Q2Classic {
                milliseconds,
                angle_shorts,
                forward_move,
                side_move,
                up_move,
                buttons,
                impulse,
                light_level,
            } => vec![
                *milliseconds,
                angle_shorts[0],
                angle_shorts[1],
                angle_shorts[2],
                *forward_move,
                *side_move,
                *up_move,
                *buttons,
                *impulse,
                *light_level,
            ],
            Self::Q2Rerelease {
                milliseconds,
                angles,
                forward_move,
                side_move,
                buttons,
                server_frame,
            } => vec![
                *milliseconds,
                angles[0],
                angles[1],
                angles[2],
                *forward_move,
                *side_move,
                *buttons,
                *server_frame,
            ],
            Self::Q3 {
                server_time_milliseconds,
                angle_words,
                buttons,
                weapon,
                forward_move,
                right_move,
                up_move,
            } => vec![
                *server_time_milliseconds,
                angle_words[0],
                angle_words[1],
                angle_words[2],
                *buttons,
                *weapon,
                *forward_move,
                *right_move,
                *up_move,
            ],
        }
    }
}

/// Arsenal intent (`ArsenalIntent`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArsenalIntent {
    /// Weapon provider.
    pub provider: String,
    /// Selected weapon, if any.
    pub weapon: Option<String>,
    /// Holdable in use.
    pub use_holdable: bool,
}

/// Authenticated command source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CommandSource {
    /// Bot client.
    Bot {
        /// Owning client.
        client: ClientId,
    },
    /// Remote client.
    Remote {
        /// Owning client.
        client: ClientId,
    },
    /// Local seat.
    LocalSeat {
        /// Owning seat.
        seat: SeatId,
    },
}

/// Actor command (`ActorCommand`).
#[derive(Debug, Clone, PartialEq)]
pub struct ActorCommand {
    /// Controlled actor.
    pub actor: ActorId,
    /// Authenticated source.
    pub source: CommandSource,
    /// Command sequence.
    pub sequence: u64,
    /// Movement command.
    pub command: UserCommand,
    /// Arsenal intent, if any.
    pub arsenal: Option<ArsenalIntent>,
}

/// Controlled actor binding (`UnifiedCommandActor`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnifiedCommandActor {
    /// Actor handle.
    pub actor: ActorId,
    /// Movement dialect.
    pub movement: MovementDialect,
    /// Arsenal provider.
    pub arsenal: ProviderId,
}

/// Authenticated command receiver (`UnifiedCommandReceiver`).
pub trait UnifiedCommandReceiver {
    /// Authenticated source; never from packet bytes.
    fn source(&self) -> CommandSource;
    /// Resolve a current actor controlled by this source.
    fn resolve_controlled_actor(&self, slot: u32, generation: u32) -> Option<UnifiedCommandActor>;
}

fn identifier(value: &str) -> Result<&str, CommandError> {
    let colon = value.find(':').ok_or(CommandError::BadIdentifier)?;
    if colon < 1 || colon == value.len() - 1 || value.contains('\0') {
        return Err(CommandError::BadIdentifier);
    }
    Ok(value)
}

fn string_bytes(value: &str) -> Result<Vec<u8>, CommandError> {
    identifier(value)?;
    let bytes = value.as_bytes().to_vec();
    if bytes.len() > 65535 {
        return Err(CommandError::IdentifierTooLong);
    }
    Ok(bytes)
}

/// Encode an actor command (`encodeUnifiedActorCommand`).
pub fn encode_unified_actor_command(input: &ActorCommand) -> Result<Vec<u8>, CommandError> {
    if input.sequence > (1u64 << 53) - 1 {
        return Err(CommandError::BadSequence);
    }
    let values = input.command.fields();
    for value in &values {
        if !value.is_finite() {
            return Err(CommandError::NonFiniteNumber);
        }
    }
    let provider = input
        .arsenal
        .as_ref()
        .map(|intent| string_bytes(&intent.provider))
        .transpose()?;
    let weapon = input
        .arsenal
        .as_ref()
        .and_then(|intent| intent.weapon.as_ref())
        .map(|weapon| string_bytes(weapon))
        .transpose()?;
    let mut writer = Vec::with_capacity(
        32 + values.len() * 8 + provider.as_ref().map_or(0, Vec::len) + weapon.as_ref().map_or(0, Vec::len),
    );
    writer.extend_from_slice(b"QTCM");
    writer.extend_from_slice(&1u16.to_le_bytes());
    writer.extend_from_slice(&input.actor.slot().to_le_bytes());
    writer.extend_from_slice(&input.actor.generation().to_le_bytes());
    writer.extend_from_slice(&(input.sequence as f64).to_le_bytes());
    writer.push(match input.command.dialect() {
        MovementDialect::Q1Netquake => 0,
        MovementDialect::Q1Quakeworld => 1,
        MovementDialect::Q2Classic => 2,
        MovementDialect::Q2Rerelease => 3,
        MovementDialect::Q3 => 4,
    });
    for value in &values {
        writer.extend_from_slice(&value.to_le_bytes());
    }
    match (&input.arsenal, &provider) {
        (None, _) => writer.push(0),
        (Some(intent), Some(provider)) => {
            writer.push(1);
            writer.extend_from_slice(&(provider.len() as u16).to_le_bytes());
            writer.extend_from_slice(provider);
            match &weapon {
                None => writer.push(0),
                Some(weapon) => {
                    writer.push(1);
                    writer.extend_from_slice(&(weapon.len() as u16).to_le_bytes());
                    writer.extend_from_slice(weapon);
                }
            }
            writer.push(u8::from(intent.use_holdable));
        }
        (Some(_), None) => return Err(CommandError::BadIdentifier),
    }
    Ok(writer)
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], CommandError> {
        if self.offset + length > self.bytes.len() {
            return Err(CommandError::Truncated(self.offset));
        }
        let slice = &self.bytes[self.offset..self.offset + length];
        self.offset += length;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8, CommandError> {
        Ok(self.take(1)?[0])
    }

    fn u16(&mut self) -> Result<u16, CommandError> {
        let bytes = self.take(2)?;
        Ok(u16::from_le_bytes([bytes[0], bytes[1]]))
    }

    fn u32(&mut self) -> Result<u32, CommandError> {
        let bytes = self.take(4)?;
        Ok(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn f64(&mut self) -> Result<f64, CommandError> {
        let bytes = self.take(8)?;
        let value = f64::from_le_bytes([
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6], bytes[7],
        ]);
        if !value.is_finite() {
            return Err(CommandError::NonFiniteNumber);
        }
        Ok(value)
    }

    fn remaining(&self) -> usize {
        self.bytes.len() - self.offset
    }
}

fn read_movement(cursor: &mut Cursor<'_>) -> Result<UserCommand, CommandError> {
    let tag = cursor.u8()?;
    let vector = |cursor: &mut Cursor<'_>| -> Result<[f64; 3], CommandError> {
        Ok([cursor.f64()?, cursor.f64()?, cursor.f64()?])
    };
    match tag {
        0 => Ok(UserCommand::Q1Netquake {
            acknowledged_server_time_seconds: cursor.f64()?,
            view_angles: vector(cursor)?,
            forward_move: cursor.f64()?,
            side_move: cursor.f64()?,
            up_move: cursor.f64()?,
            buttons: cursor.f64()?,
            impulse: cursor.f64()?,
        }),
        1 => Ok(UserCommand::Q1Quakeworld {
            milliseconds: cursor.f64()?,
            angles: vector(cursor)?,
            forward_move: cursor.f64()?,
            side_move: cursor.f64()?,
            up_move: cursor.f64()?,
            buttons: cursor.f64()?,
            impulse: cursor.f64()?,
        }),
        2 => Ok(UserCommand::Q2Classic {
            milliseconds: cursor.f64()?,
            angle_shorts: vector(cursor)?,
            forward_move: cursor.f64()?,
            side_move: cursor.f64()?,
            up_move: cursor.f64()?,
            buttons: cursor.f64()?,
            impulse: cursor.f64()?,
            light_level: cursor.f64()?,
        }),
        3 => Ok(UserCommand::Q2Rerelease {
            milliseconds: cursor.f64()?,
            angles: vector(cursor)?,
            forward_move: cursor.f64()?,
            side_move: cursor.f64()?,
            buttons: cursor.f64()?,
            server_frame: cursor.f64()?,
        }),
        4 => Ok(UserCommand::Q3 {
            server_time_milliseconds: cursor.f64()?,
            angle_words: vector(cursor)?,
            buttons: cursor.f64()?,
            weapon: cursor.f64()?,
            forward_move: cursor.f64()?,
            right_move: cursor.f64()?,
            up_move: cursor.f64()?,
        }),
        _ => Err(CommandError::BadDialect),
    }
}

/// Decode an actor command (`decodeUnifiedActorCommand`).
pub fn decode_unified_actor_command(
    bytes: &[u8],
    receiver: &dyn UnifiedCommandReceiver,
    owner: &IdentityOwner,
) -> Result<ActorCommand, CommandError> {
    let mut cursor = Cursor::new(bytes);
    if cursor.take(4)? != b"QTCM" {
        return Err(CommandError::BadMagic);
    }
    if cursor.u16()? != 1 {
        return Err(CommandError::BadVersion);
    }
    let slot = cursor.u32()?;
    let generation = cursor.u32()?;
    let sequence_value = cursor.f64()?;
    if sequence_value < 0.0 || sequence_value.fract() != 0.0 || sequence_value > ((1u64 << 53) - 1) as f64 {
        return Err(CommandError::BadSequence);
    }
    let sequence = sequence_value as u64;
    let command = read_movement(&mut cursor)?;
    let has_intent = cursor.u8()?;
    let mut arsenal = None;
    if has_intent == 1 {
        let provider_length = cursor.u16()? as usize;
        let provider_text = std::str::from_utf8(cursor.take(provider_length)?).map_err(|_| CommandError::BadText)?;
        let provider = identifier(provider_text)?.to_owned();
        let has_weapon = cursor.u8()?;
        if has_weapon != 0 && has_weapon != 1 {
            return Err(CommandError::BadWeaponTag);
        }
        let weapon = if has_weapon == 0 {
            None
        } else {
            let weapon_length = cursor.u16()? as usize;
            let weapon_text = std::str::from_utf8(cursor.take(weapon_length)?).map_err(|_| CommandError::BadText)?;
            Some(identifier(weapon_text)?.to_owned())
        };
        let use_holdable = cursor.u8()?;
        if use_holdable != 0 && use_holdable != 1 {
            return Err(CommandError::BadHoldable);
        }
        arsenal = Some(ArsenalIntent {
            provider,
            weapon,
            use_holdable: use_holdable == 1,
        });
    } else if has_intent != 0 {
        return Err(CommandError::BadIntentTag);
    }
    if cursor.remaining() != 0 {
        return Err(CommandError::TrailingBytes);
    }
    let controlled = receiver
        .resolve_controlled_actor(slot, generation)
        .ok_or(CommandError::StaleActor)?;
    if controlled.actor.slot() != slot || controlled.actor.generation() != generation {
        return Err(CommandError::StaleActor);
    }
    if !owner.owns_actor(&controlled.actor) {
        return Err(CommandError::StaleActor);
    }
    let source = receiver.source();
    let same_session = match &source {
        CommandSource::Bot { client } | CommandSource::Remote { client } => owner.owns_client(client),
        CommandSource::LocalSeat { seat } => owner.owns_seat(seat),
    };
    if !same_session {
        return Err(CommandError::ForeignSource);
    }
    if command.dialect() != controlled.movement {
        return Err(CommandError::DialectMismatch);
    }
    if let Some(intent) = &arsenal {
        let provider = ProviderId::new(
            intent.provider.split(':').next().unwrap_or(""),
            intent.provider.split(':').nth(1).unwrap_or(""),
        );
        if provider != controlled.arsenal {
            return Err(CommandError::ProviderMismatch);
        }
    }
    Ok(ActorCommand {
        actor: controlled.actor,
        source,
        sequence,
        command,
        arsenal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixture {
        source: CommandSource,
        actor: UnifiedCommandActor,
    }

    impl UnifiedCommandReceiver for Fixture {
        fn source(&self) -> CommandSource {
            self.source.clone()
        }

        fn resolve_controlled_actor(&self, slot: u32, generation: u32) -> Option<UnifiedCommandActor> {
            if self.actor.actor.slot() == slot && self.actor.actor.generation() == generation {
                Some(self.actor.clone())
            } else {
                None
            }
        }
    }

    #[test]
    fn actor_command_round_trips() {
        let owner = IdentityOwner::create("test").unwrap();
        let actor_id = owner.actor(3, 1);
        let client = owner.client(1, 1);
        let receiver = Fixture {
            source: CommandSource::Remote { client: client.clone() },
            actor: UnifiedCommandActor {
                actor: actor_id.clone(),
                movement: MovementDialect::Q3,
                arsenal: ProviderId::new("q3", "arsenal"),
            },
        };
        let input = ActorCommand {
            actor: actor_id,
            source: CommandSource::Remote { client },
            sequence: 42,
            command: UserCommand::Q3 {
                server_time_milliseconds: 1000.0,
                angle_words: [1.0, 2.0, 3.0],
                buttons: 4.0,
                weapon: 5.0,
                forward_move: 6.0,
                right_move: 7.0,
                up_move: 8.0,
            },
            arsenal: Some(ArsenalIntent {
                provider: "q3:arsenal".to_owned(),
                weapon: Some("q3:railgun".to_owned()),
                use_holdable: true,
            }),
        };
        let bytes = encode_unified_actor_command(&input).unwrap();
        assert_eq!(&bytes[0..4], b"QTCM");
        let decoded = decode_unified_actor_command(&bytes, &receiver, &owner).unwrap();
        assert_eq!(decoded.sequence, 42);
        assert_eq!(decoded.arsenal, input.arsenal);
        let mut bad = bytes.clone();
        bad.push(0);
        assert_eq!(
            decode_unified_actor_command(&bad, &receiver, &owner),
            Err(CommandError::TrailingBytes)
        );
    }
}
