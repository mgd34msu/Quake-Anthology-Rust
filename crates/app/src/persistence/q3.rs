//! Quake III session persistence ported from `src/persistence/q3.ts`.
//!
//! `g_session.c` cvar payloads plus the `bg_lib.c` integer scanner. Reads
//! and writes run through [`Q3SessionCvars`], which the shared
//! [`qa_core::cvar::CvarRegistry`] satisfies, so session state round-trips
//! through the same registry the console uses.

use std::collections::HashMap;

use super::PersistenceError;

/// Session cvar access (`session` plus one `session{slot}` per client).
pub trait Q3SessionCvars {
    /// Read a session cvar.
    fn session_get(&self, name: &str) -> Result<String, PersistenceError>;
    /// Write a session cvar.
    fn session_set(&mut self, name: &str, value: &str) -> Result<(), PersistenceError>;
}

impl Q3SessionCvars for qa_core::cvar::CvarRegistry {
    fn session_get(&self, name: &str) -> Result<String, PersistenceError> {
        Ok(self.variable_string(name))
    }

    fn session_set(&mut self, name: &str, value: &str) -> Result<(), PersistenceError> {
        self.set(name, value, true)
            .map(|_| ())
            .map_err(|error| PersistenceError::BadSave(error.to_string()))
    }
}

impl Q3SessionCvars for HashMap<String, String> {
    fn session_get(&self, name: &str) -> Result<String, PersistenceError> {
        Ok(self.get(name).cloned().unwrap_or_default())
    }

    fn session_set(&mut self, name: &str, value: &str) -> Result<(), PersistenceError> {
        self.insert(name.to_string(), value.to_string());
        Ok(())
    }
}

/// One client's persisted session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ClientSession {
    /// Team.
    pub team: i32,
    /// Spectator time.
    pub spectator_time: i32,
    /// Spectator state.
    pub spectator_state: i32,
    /// Spectator client.
    pub spectator_client: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team leader flag.
    pub team_leader: i32,
}

/// Session save: game type plus per-slot client sessions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3SessionSave {
    /// Game type.
    pub game_type: i32,
    /// Client sessions by slot.
    pub clients: Vec<Q3ClientEntry>,
}

/// One saved client session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Q3ClientEntry {
    /// Client slot.
    pub slot: u32,
    /// Session.
    pub session: Q3ClientSession,
}

fn source_buffer(value: &str) -> Result<String, PersistenceError> {
    let truncated = value.split('\0').next().unwrap_or("");
    let buffer: String = truncated.chars().take(1023).collect();
    if buffer.chars().any(|character| character as u32 > 255) {
        return Err(PersistenceError::BadSave(
            "q3-session: expected byte characters".to_string(),
        ));
    }
    Ok(buffer)
}

fn scan_integer(buffer: &str, start: usize) -> Result<(i32, usize), PersistenceError> {
    let bytes: Vec<u32> = buffer.chars().map(|character| character as u32).collect();
    if start > bytes.len() {
        return Err(PersistenceError::BadSave(
            "q3-session: source integer scan passed its terminating NUL".to_string(),
        ));
    }
    let mut cursor = start;
    while cursor < bytes.len() {
        let byte = bytes[cursor];
        let signed = if byte < 128 { byte as i32 } else { byte as i32 - 256 };
        if signed > 32 {
            break;
        }
        cursor += 1;
    }
    if cursor == bytes.len() {
        return Ok((0, cursor));
    }
    let sign = if bytes[cursor] == '-' as u32 { -1 } else { 1 };
    if bytes[cursor] == '+' as u32 || bytes[cursor] == '-' as u32 {
        cursor += 1;
    }
    let mut value: i32 = 0;
    loop {
        if cursor == bytes.len() {
            return Ok((value.wrapping_mul(sign), cursor + 1));
        }
        let byte = bytes[cursor];
        cursor += 1;
        if !(48..=57).contains(&byte) {
            return Ok((value.wrapping_mul(sign), cursor));
        }
        value = value.wrapping_mul(10).wrapping_add(byte as i32 - 48);
    }
}

/// Decode one client session payload.
pub fn decode_q3_client_session(value: &str) -> Result<Q3ClientSession, PersistenceError> {
    let buffer = source_buffer(value)?;
    let mut offset = 0;
    let mut integer = || -> Result<i32, PersistenceError> {
        let (value, next) = scan_integer(&buffer, offset)?;
        offset = next;
        Ok(value)
    };
    Ok(Q3ClientSession {
        team: integer()?,
        spectator_time: integer()?,
        spectator_state: integer()?,
        spectator_client: integer()?,
        wins: integer()?,
        losses: integer()?,
        team_leader: integer()?,
    })
}

/// Encode one client session payload.
#[must_use]
pub fn encode_q3_client_session(session: &Q3ClientSession) -> String {
    [
        session.team,
        session.spectator_time,
        session.spectator_state,
        session.spectator_client,
        session.wins,
        session.losses,
        session.team_leader,
    ]
    .iter()
    .map(|value| format!("{value}"))
    .collect::<Vec<_>>()
    .join(" ")
}

/// Read sessions from cvars.
pub fn read_q3_sessions(cvars: &impl Q3SessionCvars, client_slots: &[u32]) -> Result<Q3SessionSave, PersistenceError> {
    let game_type = scan_integer(&source_buffer(&cvars.session_get("session")?)?, 0)?.0;
    let mut clients = Vec::new();
    for slot in client_slots {
        let payload = cvars.session_get(&format!("session{slot}"))?;
        clients.push(Q3ClientEntry {
            slot: *slot,
            session: decode_q3_client_session(&payload)?,
        });
    }
    Ok(Q3SessionSave { game_type, clients })
}

/// Write sessions to cvars.
pub fn write_q3_sessions(cvars: &mut impl Q3SessionCvars, save: &Q3SessionSave) -> Result<(), PersistenceError> {
    cvars.session_set("session", &format!("{}", save.game_type))?;
    for client in &save.clients {
        cvars.session_set(
            &format!("session{}", client.slot),
            &encode_q3_client_session(&client.session),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Q3ClientSession {
        Q3ClientSession {
            team: 1,
            spectator_time: 120,
            spectator_state: 0,
            spectator_client: -1,
            wins: 3,
            losses: 2,
            team_leader: 0,
        }
    }

    #[test]
    fn sessions_round_trip_through_cvars() {
        assert_eq!(encode_q3_client_session(&session()), "1 120 0 -1 3 2 0");
        assert_eq!(decode_q3_client_session("1 120 0 -1 3 2 0").unwrap(), session());
        let mut cvars = HashMap::new();
        let save = Q3SessionSave {
            game_type: 4,
            clients: vec![
                Q3ClientEntry {
                    slot: 0,
                    session: session(),
                },
                Q3ClientEntry {
                    slot: 3,
                    session: Q3ClientSession {
                        team: 2,
                        spectator_time: 0,
                        spectator_state: 1,
                        spectator_client: 0,
                        wins: 0,
                        losses: 0,
                        team_leader: 1,
                    },
                },
            ],
        };
        write_q3_sessions(&mut cvars, &save).unwrap();
        assert_eq!(read_q3_sessions(&cvars, &[0, 3]).unwrap(), save);
    }

    #[test]
    fn scanner_matches_source_edges() {
        // Empty payload scans zeros; trailing NUL content is ignored.
        assert_eq!(decode_q3_client_session("").unwrap().wins, 0);
        assert_eq!(decode_q3_client_session("5 0 0 0 0 0 0\0ignored").unwrap().team, 5);
        // Truncated payloads fail like the source scanner.
        assert!(decode_q3_client_session("5").is_err());
        // Non-numeric tails stop the scan and keep the accumulated value.
        assert_eq!(decode_q3_client_session("12x 3 0 0 0 0 0").unwrap().team, 12);
        // Wrapping matches Math.imul overflow.
        let wrapped = decode_q3_client_session("9999999999 0 0 0 0 0 0").unwrap().team;
        assert_eq!(wrapped, 1410065407);
        assert!(source_buffer("ÿĀ").is_err());
    }
}
