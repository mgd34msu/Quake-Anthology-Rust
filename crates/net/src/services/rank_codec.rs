//! Q3 ranking ID codecs ported from `src/network/services/rank-codec.ts`.
//!
//! Port of id Software's `server/sv_rankings.c` ASCII and 64-bit ID codecs.
//! Byte behavior matches the donor, including partial group writes and the
//! leading-NUL short-destination rule.

use thiserror::Error;

const ASCII_ENCODING: &[u8; 64] = b"0123456789abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ[]";

/// Error for ranking codec failures.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RankCodecError {
    /// Ranking text is not a C byte.
    #[error("Ranking text at {0} is not a C byte")]
    NotCByte(usize),
    /// Ranking text has an undefined signed-char index.
    #[error("Ranking text at {0} has an undefined signed-char index")]
    UndefinedIndex(usize),
    /// Ranking game ID exceeds uint64 range.
    #[error("Ranking game ID exceeds uint64 range")]
    GameIdRange,
    /// Ranking player ID does not contain eight initialized bytes.
    #[error("Ranking player ID does not contain eight initialized bytes")]
    ShortPlayerId,
}

/// `SV_RankAsciiEncode`. Returns text length; also writes the NUL terminator.
///
/// Panics if `destination` cannot hold the output plus terminator, matching
/// the donor's checked writes.
#[must_use]
pub fn rank_ascii_encode(destination: &mut [u8], source: &[u8]) -> usize {
    let mut length = 0;
    let mut index = 0;
    while index < source.len() {
        let first = source[index];
        let second = source.get(index + 1).copied().unwrap_or(0);
        let third = source.get(index + 2).copied().unwrap_or(0);
        let text = [
            first >> 2,
            ((u16::from(first) << 4) | (u16::from(second) >> 4)) as u8 & 63,
            ((u16::from(second) << 2) | (u16::from(third) >> 6)) as u8 & 63,
            third & 63,
        ];
        let count = if index + 2 < source.len() {
            4
        } else {
            (source.len() - index) * 4 / 3 + 1
        };
        for value in text.iter().take(count) {
            destination[length] = ASCII_ENCODING[*value as usize];
            length += 1;
        }
        index += 3;
    }
    destination[length] = 0;
    length
}

struct DecodeResult {
    valid: bool,
    written: usize,
}

fn decode_ascii(destination: &mut [u8], source: &[u8]) -> Result<DecodeResult, RankCodecError> {
    let mut length = 0;
    let mut index = 0;
    while index < source.len() {
        let mut text = [0u8; 4];
        for (character, slot) in text.iter_mut().enumerate() {
            let position = index + character;
            let value = if position >= source.len() {
                0
            } else {
                let code = source[position];
                if code >= 128 {
                    return Err(RankCodecError::UndefinedIndex(position));
                }
                match ASCII_ENCODING.iter().position(|candidate| *candidate == code) {
                    Some(value) => value as u8,
                    None => {
                        return Ok(DecodeResult {
                            valid: false,
                            written: length,
                        })
                    }
                }
            };
            *slot = value;
        }
        let bytes = [
            (text[0] << 2) | (text[1] >> 4),
            (text[1] << 4) | (text[2] >> 2),
            (text[2] << 6) | text[3],
        ];
        let count = if index + 3 < source.len() {
            3
        } else {
            (source.len() - index) * 3 / 4
        };
        for value in bytes.iter().take(count) {
            destination[length] = *value;
            length += 1;
        }
        index += 4;
    }
    Ok(DecodeResult {
        valid: true,
        written: length,
    })
}

/// `SV_RankAsciiDecode`. Invalid text returns zero, retaining earlier group
/// writes already stored in `destination`.
pub fn rank_ascii_decode(destination: &mut [u8], source: &str) -> Result<usize, RankCodecError> {
    if !source.is_ascii() {
        let position = source.bytes().position(|byte| byte >= 128).unwrap_or(0);
        return Err(RankCodecError::NotCByte(position));
    }
    let result = decode_ascii(destination, source.as_bytes())?;
    Ok(if result.valid { result.written } else { 0 })
}

/// `SV_RankEncodeGameID`. A short destination receives only a leading NUL.
pub fn rank_encode_game_id(game_id: u64, destination: &mut [u8], mut debug_print: impl FnMut(&str)) {
    if destination.len() < 12 {
        debug_print("SV_RankEncodeGameID: result buffer too small\n");
        if !destination.is_empty() {
            destination[0] = 0;
        }
        return;
    }
    let _ = rank_ascii_encode(destination, &game_id.to_le_bytes());
}

/// `SV_RankDecodePlayerID`, rejecting native uninitialized or out-of-bounds
/// reads.
pub fn rank_decode_player_id(source: &str, mut debug_print: impl FnMut(&str)) -> Result<u64, RankCodecError> {
    let end = source.find('\0').unwrap_or(source.len());
    let text = &source[..end];
    debug_print(&format!("SV_RankDecodePlayerID: string length {}\n", text.len()));
    if !text.is_ascii() {
        let position = text.bytes().position(|byte| byte >= 128).unwrap_or(0);
        return Err(RankCodecError::NotCByte(position));
    }
    let mut buffer = [0u8; 9];
    let result = decode_ascii(&mut buffer, text.as_bytes())?;
    // The source ignores decode failure and consumes any eight bytes already
    // written.
    if result.written < 8 {
        return Err(RankCodecError::ShortPlayerId);
    }
    Ok(u64::from_le_bytes([
        buffer[0], buffer[1], buffer[2], buffer[3], buffer[4], buffer[5], buffer[6], buffer[7],
    ]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn game_id_round_trips() {
        let mut destination = [0u8; 12];
        let mut log = String::new();
        rank_encode_game_id(0x0102_0304_0506_0708, &mut destination, |text| log.push_str(text));
        assert!(log.is_empty());
        let text = std::str::from_utf8(&destination[..11]).unwrap().to_owned();
        let decoded = rank_decode_player_id(&text, |_| {}).unwrap();
        assert_eq!(decoded, 0x0102_0304_0506_0708);
    }

    #[test]
    fn short_destination_receives_nul() {
        let mut destination = [0xFFu8; 4];
        let mut log = String::new();
        rank_encode_game_id(1, &mut destination, |text| log.push_str(text));
        assert_eq!(destination[0], 0);
        assert!(log.contains("too small"));
    }

    #[test]
    fn invalid_text_decodes_zero() {
        let mut destination = [0u8; 16];
        assert_eq!(rank_ascii_decode(&mut destination, "!!!!").unwrap(), 0);
        assert!(rank_decode_player_id("short", |_| {}).is_err());
    }
}
