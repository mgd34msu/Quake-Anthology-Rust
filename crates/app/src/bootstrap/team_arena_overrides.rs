//! Team Arena per-seat timer overrides: capture, checkpoint, apply, release.
//!
//! Port of `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/team-arena-overrides.ts`
//! (`OverrideSeat`, `OverrideRegistry`, `TeamArenaOverrides`,
//! `captureTeamArenaOverrides`, `readTeamArenaOverrides`,
//! `decodeTeamArenaOverrides`, `saveTeamArenaOverrides`,
//! `applyTeamArenaOverrides`, `releaseTeamArenaOverrides`,
//! `teamArenaArchiveEntries`). The donor `SaveImage` contract (providers,
//! recipe execution modules, map entity provider) and
//! `simulationProviderCheckpoint` are absorbed as minimal local structs
//! because `qa_world::session::SaveImage` ports only the headless
//! simulation. Checkpoint encode/decode reuse
//! `qa_world::save::value`; the server override table is shared with the
//! sibling [`super::team_arena_skirmish`] port.

use qa_core::cvar::{CvarArchiveEntry, CvarError, CvarRegistry};
use qa_world::save::value::{
    arr, decode_checkpoint_value, encode_checkpoint_value, int, obj, str as json_str, SaveJson, SaveReader,
};
use thiserror::Error;

use super::team_arena_skirmish::TEAM_ARENA_SERVER_OVERRIDES;

/// Checkpoint schema for the override provider.
pub const TEAM_ARENA_OVERRIDES_SCHEMA: &str = "world:team-arena-overrides";

/// Team Arena override failure.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TeamArenaOverridesError {
    /// Saved seats do not match the local clients.
    #[error("Saved Team Arena override seats do not match local clients")]
    SeatMismatch,
    /// `ui_drawTimer` baseline is missing.
    #[error("Team Arena timer baseline is missing")]
    MissingBaseline,
    /// Checkpoint has no native Q3 owner module.
    #[error("Team Arena override checkpoint has no native Q3 owner")]
    NoNativeOwner,
    /// Saved seat is missing while writing a checkpoint.
    #[error("Missing saved override seat")]
    MissingSavedSeat,
    /// Seat is missing while applying overrides.
    #[error("Missing override seat")]
    MissingSeat,
    /// Server archive baseline is missing.
    #[error("Team Arena source archive baseline is missing")]
    MissingArchiveBaseline,
    /// Checkpoint decode failure (donor `SaveReader` text).
    #[error("{0}")]
    Save(String),
    /// Cvar write failure.
    #[error(transparent)]
    Cvar(#[from] CvarError),
}

/// One seat/client pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct OverrideSeat {
    /// Seat index.
    pub seat: u32,
    /// Client slot.
    pub client: u32,
}

/// One seat/client pair with its cvar registry.
pub struct OverrideRegistry<'a> {
    /// Seat index.
    pub seat: u32,
    /// Client slot.
    pub client: u32,
    /// Seat cvars.
    pub cvars: &'a mut CvarRegistry,
}

impl OverrideRegistry<'_> {
    /// Seat/client pair.
    #[must_use]
    pub fn seat(&self) -> OverrideSeat {
        OverrideSeat {
            seat: self.seat,
            client: self.client,
        }
    }
}

/// Override lifecycle phase (donor `"active" | "released"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OverridePhase {
    /// Overrides are applied.
    Active,
    /// Baselines were restored.
    Released,
}

impl OverridePhase {
    /// Donor spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Released => "released",
        }
    }
}

/// One saved seat row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideSeatState {
    /// Seat index.
    pub seat: u32,
    /// Client slot.
    pub client: u32,
    /// `ui_drawTimer` baseline value.
    pub baseline: String,
    /// Effective `cg_drawTimer` value.
    pub effective: String,
}

/// Captured per-seat timer overrides.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TeamArenaOverrides {
    /// Lifecycle phase.
    pub phase: OverridePhase,
    /// Saved seat rows.
    pub seats: Vec<OverrideSeatState>,
}

/// Absorbed provider checkpoint (donor `SaveImage` provider row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideProviderCheckpoint {
    /// Owning provider.
    pub provider: String,
    /// Checkpoint schema.
    pub schema: String,
    /// Schema version.
    pub version: u32,
    /// Encoded checkpoint bytes.
    pub bytes: Vec<u8>,
}

/// Absorbed recipe execution module (donor `SaveImage` recipe row).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideExecutionModule {
    /// Module kind (donor `"typescript"` for script owners).
    pub kind: String,
    /// Module API kind (donor `"q3-qagame"` for native Q3 owners).
    pub api_kind: String,
}

/// Absorbed save image: provider checkpoints plus the recipe execution
/// list and map entity provider the donor reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverrideSaveImage {
    /// Provider checkpoints.
    pub providers: Vec<OverrideProviderCheckpoint>,
    /// Recipe execution modules.
    pub execution: Vec<OverrideExecutionModule>,
    /// Map entity provider for newly written checkpoints.
    pub map_provider: String,
}

fn require_seats(state: &TeamArenaOverrides, expected: &[OverrideSeat]) -> Result<(), TeamArenaOverridesError> {
    let mut seats: Vec<u32> = state.seats.iter().map(|row| row.seat).collect();
    seats.sort_unstable();
    seats.dedup();
    let mut clients: Vec<u32> = state.seats.iter().map(|row| row.client).collect();
    clients.sort_unstable();
    clients.dedup();
    let matches = state.seats.len() == expected.len()
        && seats.len() == state.seats.len()
        && clients.len() == state.seats.len()
        && state.seats.iter().all(|row| {
            expected
                .iter()
                .any(|seat| seat.seat == row.seat && seat.client == row.client)
        });
    if matches {
        Ok(())
    } else {
        Err(TeamArenaOverridesError::SeatMismatch)
    }
}

fn expected_seats(seats: &[OverrideRegistry]) -> Vec<OverrideSeat> {
    seats.iter().map(OverrideRegistry::seat).collect()
}

/// Capture the live timer baseline plus effective value for each seat.
pub fn capture_team_arena_overrides(seats: &[OverrideRegistry]) -> Result<TeamArenaOverrides, TeamArenaOverridesError> {
    let mut rows = Vec::with_capacity(seats.len());
    for row in seats {
        let baseline = row
            .cvars
            .get("ui_drawTimer")
            .ok_or(TeamArenaOverridesError::MissingBaseline)?;
        rows.push(OverrideSeatState {
            seat: row.seat,
            client: row.client,
            baseline: baseline.value,
            effective: row.cvars.variable_string("cg_drawTimer"),
        });
    }
    let state = TeamArenaOverrides {
        phase: OverridePhase::Active,
        seats: rows,
    };
    require_seats(&state, &expected_seats(seats))?;
    Ok(state)
}

/// Read overrides from a save image, or `None` when no override
/// checkpoint is present.
pub fn read_team_arena_overrides(
    image: &OverrideSaveImage,
    seats: &[OverrideSeat],
) -> Result<Option<TeamArenaOverrides>, TeamArenaOverridesError> {
    if !image
        .providers
        .iter()
        .any(|row| row.schema == TEAM_ARENA_OVERRIDES_SCHEMA)
    {
        return Ok(None);
    }
    if !image
        .execution
        .iter()
        .any(|module| module.kind == "typescript" && module.api_kind == "q3-qagame")
    {
        return Err(TeamArenaOverridesError::NoNativeOwner);
    }
    let checkpoint = image
        .providers
        .iter()
        .find(|row| row.schema == TEAM_ARENA_OVERRIDES_SCHEMA)
        .expect("override checkpoint presence was checked above");
    let value =
        decode_checkpoint_value(&checkpoint.bytes).map_err(|error| TeamArenaOverridesError::Save(error.to_string()))?;
    decode_team_arena_overrides(&value, seats).map(Some)
}

/// Decode an override checkpoint value.
pub fn decode_team_arena_overrides(
    value: &SaveJson,
    seats: &[OverrideSeat],
) -> Result<TeamArenaOverrides, TeamArenaOverridesError> {
    let reader = SaveReader::at(value, "Team Arena overrides");
    let phase = reader
        .field("phase")
        .string()
        .map_err(|error| TeamArenaOverridesError::Save(error.to_string()))?;
    let phase = match phase.as_str() {
        "active" => OverridePhase::Active,
        "released" => OverridePhase::Released,
        _ => {
            return Err(TeamArenaOverridesError::Save(
                reader.fail("invalid override phase").to_string(),
            ))
        }
    };
    let rows = reader
        .field("seats")
        .list(|row| {
            let seat = row.field("seat").integer(0)?;
            let client = row.field("client").integer(0)?;
            let baseline = row.field("baseline").string()?;
            let effective = row.field("effective").string()?;
            Ok(OverrideSeatState {
                // Out-of-range ids cannot match a local seat, so they
                // saturate and fail seat matching below like the donor.
                seat: u32::try_from(seat).unwrap_or(u32::MAX),
                client: u32::try_from(client).unwrap_or(u32::MAX),
                baseline,
                effective,
            })
        })
        .map_err(|error: qa_world::WorldError| TeamArenaOverridesError::Save(error.to_string()))?;
    let state = TeamArenaOverrides { phase, seats: rows };
    require_seats(&state, seats)?;
    Ok(state)
}

/// Append an override checkpoint to a save image, refreshing each
/// effective value from the live seat cvars.
pub fn save_team_arena_overrides(
    image: &OverrideSaveImage,
    state: Option<&TeamArenaOverrides>,
    seats: &[OverrideRegistry],
) -> Result<OverrideSaveImage, TeamArenaOverridesError> {
    let Some(state) = state else {
        return Ok(image.clone());
    };
    require_seats(state, &expected_seats(seats))?;
    let mut rows = Vec::with_capacity(state.seats.len());
    for row in &state.seats {
        let seat = seats
            .iter()
            .find(|seat| seat.seat == row.seat && seat.client == row.client)
            .ok_or(TeamArenaOverridesError::MissingSavedSeat)?;
        rows.push(obj(vec![
            ("seat", int(i64::from(row.seat))),
            ("client", int(i64::from(row.client))),
            ("baseline", json_str(&row.baseline)),
            ("effective", json_str(&seat.cvars.variable_string("cg_drawTimer"))),
        ]));
    }
    let saved = obj(vec![("phase", json_str(state.phase.as_str())), ("seats", arr(rows))]);
    let mut next = image.clone();
    next.providers.push(OverrideProviderCheckpoint {
        provider: image.map_provider.clone(),
        schema: TEAM_ARENA_OVERRIDES_SCHEMA.to_string(),
        version: 1,
        bytes: encode_checkpoint_value(&saved),
    });
    Ok(next)
}

/// Apply captured overrides to the live seat cvars.
pub fn apply_team_arena_overrides(
    state: Option<&TeamArenaOverrides>,
    seats: &mut [OverrideRegistry],
) -> Result<(), TeamArenaOverridesError> {
    let Some(state) = state else {
        return Ok(());
    };
    require_seats(state, &expected_seats(seats))?;
    if state.phase == OverridePhase::Released {
        return Ok(());
    }
    for row in &state.seats {
        let seat = seats
            .iter_mut()
            .find(|seat| seat.seat == row.seat && seat.client == row.client)
            .ok_or(TeamArenaOverridesError::MissingSeat)?;
        seat.cvars.set("ui_drawTimer", &row.baseline, true)?;
        seat.cvars.set("cg_drawTimer", &row.effective, true)?;
    }
    Ok(())
}

/// Restore baselines and mark the overrides released.
pub fn release_team_arena_overrides(
    state: Option<TeamArenaOverrides>,
    seats: &mut [OverrideRegistry],
) -> Result<Option<TeamArenaOverrides>, TeamArenaOverridesError> {
    let Some(mut state) = state else {
        return Ok(None);
    };
    if state.phase == OverridePhase::Released {
        return Ok(Some(state));
    }
    require_seats(&state, &expected_seats(seats))?;
    for row in &state.seats {
        let seat = seats
            .iter_mut()
            .find(|seat| seat.seat == row.seat && seat.client == row.client)
            .ok_or(TeamArenaOverridesError::MissingSeat)?;
        seat.cvars.set("cg_drawTimer", &row.baseline, true)?;
    }
    state.phase = OverridePhase::Released;
    Ok(Some(state))
}

/// Archive entries with override values masked back to their baselines:
/// a per-seat `cg_drawTimer` baseline when `seat` is set, else the
/// server override baselines.
pub fn team_arena_archive_entries(
    registry: &CvarRegistry,
    state: Option<&TeamArenaOverrides>,
    seat: Option<u32>,
) -> Result<Vec<CvarArchiveEntry>, TeamArenaOverridesError> {
    let entries = registry.archive_entries(&|_| true);
    let Some(state) = state else {
        return Ok(entries);
    };
    if state.phase != OverridePhase::Active {
        return Ok(entries);
    }
    if let Some(seat) = seat {
        let saved = state.seats.iter().find(|row| row.seat == seat);
        return Ok(entries
            .into_iter()
            .map(|entry| {
                if entry.name.to_lowercase() == "cg_drawtimer" {
                    if let Some(saved) = saved {
                        return CvarArchiveEntry {
                            name: entry.name,
                            value: saved.baseline.clone(),
                        };
                    }
                }
                entry
            })
            .collect());
    }
    let mut masked = Vec::with_capacity(entries.len());
    for entry in entries {
        let setting = TEAM_ARENA_SERVER_OVERRIDES
            .iter()
            .find(|setting| setting.name.to_lowercase() == entry.name.to_lowercase());
        let saved = setting.and_then(|setting| registry.get(setting.saved));
        if setting.is_some() && saved.is_none() {
            return Err(TeamArenaOverridesError::MissingArchiveBaseline);
        }
        if let Some(saved) = saved {
            masked.push(CvarArchiveEntry {
                name: entry.name,
                value: saved.value,
            });
        } else {
            masked.push(entry);
        }
    }
    Ok(masked)
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::cmd::Dialect;

    fn seat_registry(baseline: &str, effective: &str) -> CvarRegistry {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("ui_drawTimer", baseline, 0).unwrap();
        cvars.register("cg_drawTimer", baseline, 0).unwrap();
        cvars.set("cg_drawTimer", effective, true).unwrap();
        cvars
    }

    fn image() -> OverrideSaveImage {
        OverrideSaveImage {
            providers: Vec::new(),
            execution: vec![OverrideExecutionModule {
                kind: "typescript".to_string(),
                api_kind: "q3-qagame".to_string(),
            }],
            map_provider: "q3:official".to_string(),
        }
    }

    #[test]
    fn captures_baseline_and_effective() {
        let mut cvars = seat_registry("0", "1");
        let seats = [OverrideRegistry {
            seat: 0,
            client: 1,
            cvars: &mut cvars,
        }];
        let state = capture_team_arena_overrides(&seats).unwrap();
        assert_eq!(state.phase, OverridePhase::Active);
        assert_eq!(
            state.seats,
            vec![OverrideSeatState {
                seat: 0,
                client: 1,
                baseline: "0".to_string(),
                effective: "1".to_string(),
            }]
        );
    }

    #[test]
    fn capture_requires_baseline() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars.register("cg_drawTimer", "1", 0).unwrap();
        let seats = [OverrideRegistry {
            seat: 0,
            client: 0,
            cvars: &mut cvars,
        }];
        assert_eq!(
            capture_team_arena_overrides(&seats).unwrap_err(),
            TeamArenaOverridesError::MissingBaseline
        );
    }

    #[test]
    fn capture_rejects_duplicate_seats() {
        let mut first = seat_registry("0", "1");
        let mut second = seat_registry("0", "1");
        let seats = [
            OverrideRegistry {
                seat: 0,
                client: 0,
                cvars: &mut first,
            },
            OverrideRegistry {
                seat: 0,
                client: 1,
                cvars: &mut second,
            },
        ];
        assert_eq!(
            capture_team_arena_overrides(&seats).unwrap_err(),
            TeamArenaOverridesError::SeatMismatch
        );
    }

    #[test]
    fn checkpoint_round_trip() {
        let mut cvars = seat_registry("0", "1");
        let seats = [OverrideRegistry {
            seat: 2,
            client: 3,
            cvars: &mut cvars,
        }];
        let state = capture_team_arena_overrides(&seats).unwrap();
        let saved = save_team_arena_overrides(&image(), Some(&state), &seats).unwrap();
        assert_eq!(saved.providers.len(), 1);
        assert_eq!(saved.providers[0].schema, TEAM_ARENA_OVERRIDES_SCHEMA);
        assert_eq!(saved.providers[0].version, 1);
        assert_eq!(saved.providers[0].provider, "q3:official");
        let expected = [OverrideSeat { seat: 2, client: 3 }];
        let read = read_team_arena_overrides(&saved, &expected).unwrap().unwrap();
        assert_eq!(read, state);
    }

    #[test]
    fn read_returns_none_without_checkpoint() {
        let expected = [OverrideSeat { seat: 0, client: 0 }];
        assert!(read_team_arena_overrides(&image(), &expected).unwrap().is_none());
    }

    #[test]
    fn read_requires_native_owner() {
        let mut cvars = seat_registry("0", "1");
        let seats = [OverrideRegistry {
            seat: 0,
            client: 0,
            cvars: &mut cvars,
        }];
        let state = capture_team_arena_overrides(&seats).unwrap();
        let saved = save_team_arena_overrides(&image(), Some(&state), &seats).unwrap();
        let image = OverrideSaveImage {
            execution: vec![OverrideExecutionModule {
                kind: "typescript".to_string(),
                api_kind: "q2-game".to_string(),
            }],
            ..saved
        };
        assert_eq!(
            read_team_arena_overrides(&image, &[OverrideSeat { seat: 0, client: 0 }]).unwrap_err(),
            TeamArenaOverridesError::NoNativeOwner
        );
    }

    #[test]
    fn decode_rejects_bad_phase_and_seats() {
        let value = obj(vec![("phase", json_str("stale")), ("seats", arr(Vec::new()))]);
        let error = decode_team_arena_overrides(&value, &[]).unwrap_err().to_string();
        assert!(error.contains("invalid override phase"), "{error}");

        let mut cvars = seat_registry("0", "1");
        let seats = [OverrideRegistry {
            seat: 0,
            client: 0,
            cvars: &mut cvars,
        }];
        let state = capture_team_arena_overrides(&seats).unwrap();
        let saved = save_team_arena_overrides(&image(), Some(&state), &seats).unwrap();
        let value = decode_checkpoint_value(&saved.providers[0].bytes).unwrap();
        assert_eq!(
            decode_team_arena_overrides(&value, &[OverrideSeat { seat: 9, client: 0 }]).unwrap_err(),
            TeamArenaOverridesError::SeatMismatch
        );
    }

    #[test]
    fn apply_and_release_cycle() {
        let mut cvars = seat_registry("0", "1");
        let seats = [OverrideRegistry {
            seat: 0,
            client: 0,
            cvars: &mut cvars,
        }];
        let state = capture_team_arena_overrides(&seats).unwrap();

        let mut cvars = seat_registry("7", "7");
        let mut seats = [OverrideRegistry {
            seat: 0,
            client: 0,
            cvars: &mut cvars,
        }];
        apply_team_arena_overrides(Some(&state), &mut seats).unwrap();
        assert_eq!(seats[0].cvars.variable_string("ui_drawTimer"), "0");
        assert_eq!(seats[0].cvars.variable_string("cg_drawTimer"), "1");

        let released = release_team_arena_overrides(Some(state), &mut seats).unwrap().unwrap();
        assert_eq!(released.phase, OverridePhase::Released);
        assert_eq!(seats[0].cvars.variable_string("cg_drawTimer"), "0");

        // Released states apply and release as no-ops.
        apply_team_arena_overrides(Some(&released), &mut seats).unwrap();
        let again = release_team_arena_overrides(Some(released.clone()), &mut seats)
            .unwrap()
            .unwrap();
        assert_eq!(again, released);
        apply_team_arena_overrides(None, &mut seats).unwrap();
        assert!(release_team_arena_overrides(None, &mut seats).unwrap().is_none());
    }

    #[test]
    fn archive_masks_seat_and_server_values() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars
            .register("cg_drawTimer", "1", qa_core::cvar::flags::ARCHIVE)
            .unwrap();
        cvars
            .register("capturelimit", "5", qa_core::cvar::flags::ARCHIVE)
            .unwrap();
        cvars.register("ui_saveCaptureLimit", "8", 0).unwrap();
        let state = TeamArenaOverrides {
            phase: OverridePhase::Active,
            seats: vec![OverrideSeatState {
                seat: 0,
                client: 0,
                baseline: "0".to_string(),
                effective: "1".to_string(),
            }],
        };
        let masked = team_arena_archive_entries(&cvars, Some(&state), Some(0)).unwrap();
        let timer = masked.iter().find(|entry| entry.name == "cg_drawTimer").unwrap();
        assert_eq!(timer.value, "0");

        let server = team_arena_archive_entries(&cvars, Some(&state), None).unwrap();
        let limit = server.iter().find(|entry| entry.name == "capturelimit").unwrap();
        assert_eq!(limit.value, "8");

        let released = TeamArenaOverrides {
            phase: OverridePhase::Released,
            seats: state.seats.clone(),
        };
        let plain = team_arena_archive_entries(&cvars, Some(&released), Some(0)).unwrap();
        let timer = plain.iter().find(|entry| entry.name == "cg_drawTimer").unwrap();
        assert_eq!(timer.value, "1");
    }

    #[test]
    fn archive_requires_server_baseline() {
        let mut cvars = CvarRegistry::new(Dialect::Q3);
        cvars
            .register("fraglimit", "10", qa_core::cvar::flags::ARCHIVE)
            .unwrap();
        let state = TeamArenaOverrides {
            phase: OverridePhase::Active,
            seats: Vec::new(),
        };
        // Empty seats fail matching first, so exercise the baseline path
        // through a registry missing ui_saveFragLimit with no seat rows.
        let _ = team_arena_archive_entries(&cvars, None, None).unwrap();
        assert_eq!(
            team_arena_archive_entries(&cvars, Some(&state), None).unwrap_err(),
            TeamArenaOverridesError::MissingArchiveBaseline
        );
    }
}
