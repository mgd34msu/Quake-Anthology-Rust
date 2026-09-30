//! Mod client pending-command checkpoint. Port of
//! `src/app/bootstrap/simulation/mod-client-checkpoint.ts`.

use std::collections::HashSet;

use qa_core::identity::{ActorId, ClientId, IdentityOwner, SavedActorId, SeatId};
use qa_core::time::SourceTime;
use qa_net::common::commands::UserCommand;
use qa_world::save::records::{read_saved_actor, write_saved_actor};
use qa_world::save::shared::{read_time, read_vector, write_time};
use qa_world::save::value::{
    SaveJson, SaveReader, arr, boolean, int, namespaced, num, obj, str as json_str,
};

/// Mod client arsenal selection.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientArsenal {
    pub provider: String,
    pub weapon: Option<String>,
    pub use_holdable: bool,
    pub impulse: Option<i64>,
}

/// Command source.
#[derive(Debug, Clone, PartialEq)]
pub enum CommandSource {
    LocalSeat { client: ClientId, seat: SeatId },
    RemoteClient { client: ClientId },
    Bot { provider: String },
}

/// Command angle space (always absolute on restore).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AngleSpace {
    Absolute,
}

/// One pending mod client input.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientInput {
    pub actor: ActorId,
    pub source: CommandSource,
    pub sequence: i64,
    pub angle_space: AngleSpace,
    pub command: UserCommand,
    pub arsenal: Option<ModClientArsenal>,
}

/// Donor `Number.MIN_SAFE_INTEGER`, the default `integer()` minimum.
const MIN_SAFE_INTEGER: i64 = -9_007_199_254_740_991;

/// One pending mod client command.
#[derive(Debug, Clone, PartialEq)]
pub struct ModClientCommand {
    pub input: ModClientInput,
    pub time: SourceTime,
}

/// Restore host: identity authority plus live actor/client mapping.
pub trait ModClientHost {
    fn identity(&self) -> &IdentityOwner;
    fn actor(&self, saved: SavedActorId) -> Result<ActorId, ModClientCheckpointError>;
    fn client(&self, actor: &ActorId) -> Option<ClientId>;
}

/// Mod client checkpoint failure.
#[derive(Debug, thiserror::Error)]
pub enum ModClientCheckpointError {
    #[error(transparent)]
    World(#[from] qa_world::WorldError),
}

/// Capture one pending command for `client`.
#[must_use]
pub fn capture_mod_client_command(value: &ModClientCommand, client: &ClientId) -> SaveJson {
    let source = match &value.input.source {
        CommandSource::LocalSeat { seat, .. } => {
            obj(vec![("kind", json_str("local-seat")), ("seat", int(i64::from(seat.index())))])
        }
        CommandSource::RemoteClient { .. } => obj(vec![("kind", json_str("remote-client"))]),
        CommandSource::Bot { provider } => {
            obj(vec![("kind", json_str("bot")), ("provider", json_str(provider))])
        }
    };
    obj(vec![
        ("actor", write_saved_actor(SavedActorId::from(&value.input.actor))),
        (
            "client",
            obj(vec![
                ("slot", int(i64::from(client.slot()))),
                ("generation", int(i64::from(client.generation()))),
            ]),
        ),
        ("time", write_time(value.input_time())),
        ("source", source),
        ("sequence", int(value.input.sequence)),
        ("command", write_user_command(&value.input.command)),
        (
            "arsenal",
            value.input.arsenal.as_ref().map_or(SaveJson::Null, |arsenal| {
                let mut members = vec![
                    ("provider", json_str(&arsenal.provider)),
                    (
                        "weapon",
                        arsenal.weapon.as_ref().map_or(SaveJson::Null, |weapon| json_str(weapon)),
                    ),
                    ("useHoldable", boolean(arsenal.use_holdable)),
                ];
                if let Some(impulse) = arsenal.impulse {
                    members.push(("impulse", int(impulse)));
                }
                obj(members)
            }),
        ),
    ])
}

impl ModClientCommand {
    fn input_time(&self) -> SourceTime {
        self.time
    }
}

fn write_vector(value: [f64; 3]) -> SaveJson {
    obj(vec![("x", num(value[0])), ("y", num(value[1])), ("z", num(value[2]))])
}

fn write_words(value: [f64; 3]) -> SaveJson {
    #[allow(clippy::cast_possible_truncation)]
    arr(value.into_iter().map(|word| int(word as i64)).collect())
}

fn write_user_command(command: &UserCommand) -> SaveJson {
    match command {
        UserCommand::Q1Netquake {
            acknowledged_server_time_seconds,
            view_angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => obj(vec![
            ("kind", json_str("q1-netquake")),
            ("acknowledgedServerTimeSeconds", num(*acknowledged_server_time_seconds)),
            ("viewAngles", write_vector(*view_angles)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("upMove", num(*up_move)),
            ("buttons", num(*buttons)),
            ("impulse", num(*impulse)),
        ]),
        UserCommand::Q1Quakeworld {
            milliseconds,
            angles,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
        } => obj(vec![
            ("kind", json_str("q1-quakeworld")),
            ("milliseconds", num(*milliseconds)),
            ("angles", write_vector(*angles)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("upMove", num(*up_move)),
            ("buttons", num(*buttons)),
            ("impulse", num(*impulse)),
        ]),
        UserCommand::Q2Classic {
            milliseconds,
            angle_shorts,
            forward_move,
            side_move,
            up_move,
            buttons,
            impulse,
            light_level,
        } => obj(vec![
            ("kind", json_str("q2-classic")),
            ("milliseconds", num(*milliseconds)),
            ("angleShorts", write_words(*angle_shorts)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("upMove", num(*up_move)),
            ("buttons", num(*buttons)),
            ("impulse", num(*impulse)),
            ("lightLevel", num(*light_level)),
        ]),
        UserCommand::Q2Rerelease {
            milliseconds,
            angles,
            forward_move,
            side_move,
            buttons,
            server_frame,
        } => obj(vec![
            ("kind", json_str("q2-rerelease")),
            ("milliseconds", num(*milliseconds)),
            ("angles", write_vector(*angles)),
            ("forwardMove", num(*forward_move)),
            ("sideMove", num(*side_move)),
            ("buttons", num(*buttons)),
            ("serverFrame", num(*server_frame)),
        ]),
        UserCommand::Q3 {
            server_time_milliseconds,
            angle_words,
            buttons,
            weapon,
            forward_move,
            right_move,
            up_move,
        } => obj(vec![
            ("kind", json_str("q3")),
            ("serverTimeMilliseconds", num(*server_time_milliseconds)),
            ("angleWords", write_words(*angle_words)),
            ("buttons", num(*buttons)),
            ("weapon", num(*weapon)),
            ("forwardMove", num(*forward_move)),
            ("rightMove", num(*right_move)),
            ("upMove", num(*up_move)),
        ]),
    }
}

fn read_words(reader: SaveReader) -> Result<[f64; 3], ModClientCheckpointError> {
    let values = reader.list(|value| value.integer(i64::MIN))?;
    if values.len() != 3 {
        return Err(reader.fail("Expected three command angle words").into());
    }
    #[allow(clippy::cast_precision_loss)]
    Ok([values[0] as f64, values[1] as f64, values[2] as f64])
}

fn read_angles(reader: SaveReader) -> Result<[f64; 3], ModClientCheckpointError> {
    let vector = read_vector(reader)?;
    Ok([f64::from(vector.x), f64::from(vector.y), f64::from(vector.z)])
}

/// Read one movement command.
pub fn read_user_command(reader: SaveReader) -> Result<UserCommand, ModClientCheckpointError> {
    let kind =
        reader.field("kind").choice_str(&["q1-netquake", "q1-quakeworld", "q2-classic", "q2-rerelease", "q3"])?;
    let buttons = reader.field("buttons").integer(0)? as f64;
    let forward_move = reader.field("forwardMove").finite()?;
    if kind == "q3" {
        return Ok(UserCommand::Q3 {
            buttons,
            forward_move,
            server_time_milliseconds: reader.field("serverTimeMilliseconds").integer(MIN_SAFE_INTEGER)? as f64,
            angle_words: read_words(reader.field("angleWords"))?,
            weapon: reader.field("weapon").integer(MIN_SAFE_INTEGER)? as f64,
            right_move: reader.field("rightMove").finite()?,
            up_move: reader.field("upMove").finite()?,
        });
    }
    let side_move = reader.field("sideMove").finite()?;
    if kind == "q2-rerelease" {
        return Ok(UserCommand::Q2Rerelease {
            buttons,
            forward_move,
            side_move,
            milliseconds: reader.field("milliseconds").finite()?,
            angles: read_angles(reader.field("angles"))?,
            server_frame: reader.field("serverFrame").integer(MIN_SAFE_INTEGER)? as f64,
        });
    }
    let up_move = reader.field("upMove").finite()?;
    let impulse = reader.field("impulse").integer(0)? as f64;
    if kind == "q1-netquake" {
        return Ok(UserCommand::Q1Netquake {
            buttons,
            forward_move,
            side_move,
            up_move,
            impulse,
            acknowledged_server_time_seconds: reader.field("acknowledgedServerTimeSeconds").finite()?,
            view_angles: read_angles(reader.field("viewAngles"))?,
        });
    }
    let milliseconds = reader.field("milliseconds").finite()?;
    if kind == "q1-quakeworld" {
        return Ok(UserCommand::Q1Quakeworld {
            buttons,
            forward_move,
            side_move,
            up_move,
            impulse,
            milliseconds,
            angles: read_angles(reader.field("angles"))?,
        });
    }
    Ok(UserCommand::Q2Classic {
        buttons,
        forward_move,
        side_move,
        up_move,
        impulse,
        milliseconds,
        angle_shorts: read_words(reader.field("angleShorts"))?,
        light_level: reader.field("lightLevel").integer(0)? as f64,
    })
}

/// Read pending mod client commands; a missing checkpoint restores to empty.
pub fn read_mod_client_commands(
    reader: SaveReader,
    host: &dyn ModClientHost,
) -> Result<Vec<ModClientCommand>, ModClientCheckpointError> {
    if reader.is_missing() {
        return Ok(Vec::new());
    }
    let mut seen: HashSet<ActorId> = HashSet::new();
    reader.list(|entry| -> Result<ModClientCommand, ModClientCheckpointError> {
        let actor = host.actor(read_saved_actor(entry.field("actor"))?)?;
        let client = host.client(&actor);
        let saved_client = entry.field("client");
        saved_client.field("generation").integer(0)?;
        let slot = saved_client.field("slot").integer(0)?;
        let (Some(client), Ok(slot)) = (client, u32::try_from(slot)) else {
            return Err(entry.fail("Saved command requires one live restored client").into());
        };
        if client.slot() != slot || seen.contains(&actor) {
            return Err(entry.fail("Saved command requires one live restored client").into());
        }
        seen.insert(actor.clone());
        let owner = entry.field("source");
        let kind = owner.field("kind").choice_str(&["local-seat", "remote-client", "bot"])?;
        let source = if kind == "local-seat" {
            let seat = owner.field("seat").integer(0)?;
            let seat = u32::try_from(seat)
                .map_err(|_| owner.field("seat").fail("expected an integer in range"))?;
            CommandSource::LocalSeat { client: client.clone(), seat: host.identity().seat(seat) }
        } else if kind == "remote-client" {
            CommandSource::RemoteClient { client: client.clone() }
        } else {
            CommandSource::Bot { provider: namespaced(owner.field("provider"))? }
        };
        let arsenal = entry.field("arsenal").nullable(|value| -> Result<ModClientArsenal, ModClientCheckpointError> {
            let impulse = if value.field("impulse").is_missing() {
                None
            } else {
                Some(value.field("impulse").integer(0)?)
            };
            Ok(ModClientArsenal {
                provider: namespaced(value.field("provider"))?,
                weapon: value.field("weapon").nullable(namespaced)?,
                use_holdable: value.field("useHoldable").boolean()?,
                impulse,
            })
        })?;
        Ok(ModClientCommand {
            time: read_time(entry.field("time"))?,
            input: ModClientInput {
                actor,
                source,
                sequence: entry.field("sequence").integer(0)?,
                angle_space: AngleSpace::Absolute,
                command: read_user_command(entry.field("command"))?,
                arsenal,
            },
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;

    struct MockHost {
        identity: IdentityOwner,
    }

    impl MockHost {
        fn new() -> Self {
            Self { identity: IdentityOwner::create("test").unwrap() }
        }
    }

    impl ModClientHost for MockHost {
        fn identity(&self) -> &IdentityOwner {
            &self.identity
        }
        fn actor(&self, saved: SavedActorId) -> Result<ActorId, ModClientCheckpointError> {
            Ok(self.identity.actor(saved.slot, saved.generation))
        }
        fn client(&self, actor: &ActorId) -> Option<ClientId> {
            (actor.slot() == 1).then(|| self.identity.client(2, 3))
        }
    }

    fn command() -> ModClientCommand {
        let owner = IdentityOwner::create("capture").unwrap();
        ModClientCommand {
            time: SourceTime::Seconds(4.0),
            input: ModClientInput {
                actor: owner.actor(1, 1),
                source: CommandSource::Bot { provider: "q2:bot".to_string() },
                sequence: 9,
                angle_space: AngleSpace::Absolute,
                command: UserCommand::Q2Rerelease {
                    milliseconds: 16.0,
                    angles: [1.0, 2.0, 3.0],
                    forward_move: 100.0,
                    side_move: 0.0,
                    buttons: 1.0,
                    server_frame: 44.0,
                },
                arsenal: Some(ModClientArsenal {
                    provider: "q2:game".to_string(),
                    weapon: None,
                    use_holdable: true,
                    impulse: None,
                }),
            },
        }
    }

    #[test]
    fn missing_checkpoint_restores_empty() {
        let host = MockHost::new();
        let empty = obj(vec![]);
        let reader = SaveReader::at(&empty, "root").field("absent");
        assert!(read_mod_client_commands(reader, &host).unwrap().is_empty());
    }

    #[test]
    fn capture_read_round_trip() {
        let host = MockHost::new();
        let client = host.identity.client(2, 3);
        let value = arr(vec![capture_mod_client_command(&command(), &client)]);
        let restored = read_mod_client_commands(SaveReader::at(&value, "commands"), &host).unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].input.sequence, 9);
        assert_eq!(restored[0].time.as_seconds_f64(), 4.0);
        assert!(matches!(
            restored[0].input.command,
            UserCommand::Q2Rerelease { server_frame: 44.0, .. }
        ));
    }

    #[test]
    fn command_without_live_client_is_rejected() {
        let host = MockHost::new();
        let owner = IdentityOwner::create("capture").unwrap();
        let client = host.identity.client(2, 3);
        let mut other = command();
        other.input.actor = owner.actor(7, 1);
        let value = arr(vec![capture_mod_client_command(&other, &client)]);
        let error = read_mod_client_commands(SaveReader::at(&value, "commands"), &host)
            .unwrap_err()
            .to_string();
        assert!(error.contains("Saved command requires one live restored client"), "{error}");
    }

    #[test]
    fn q3_command_parses() {
        let value = obj(vec![
            ("kind", json_str("q3")),
            ("buttons", int(1)),
            ("forwardMove", num(127.0)),
            ("serverTimeMilliseconds", int(5000)),
            ("angleWords", arr(vec![int(0), int(32768), int(0)])),
            ("weapon", int(3)),
            ("rightMove", num(0.0)),
            ("upMove", num(0.0)),
        ]);
        let command = read_user_command(SaveReader::at(&value, "command")).unwrap();
        assert!(matches!(
            command,
            UserCommand::Q3 { server_time_milliseconds: 5000.0, weapon: 3.0, .. }
        ));
    }

    #[test]
    fn short_angle_words_are_rejected() {
        let value = obj(vec![
            ("kind", json_str("q3")),
            ("buttons", int(0)),
            ("forwardMove", num(0.0)),
            ("serverTimeMilliseconds", int(0)),
            ("angleWords", arr(vec![int(0), int(0)])),
            ("weapon", int(0)),
            ("rightMove", num(0.0)),
            ("upMove", num(0.0)),
        ]);
        let error = read_user_command(SaveReader::at(&value, "command")).unwrap_err().to_string();
        assert!(error.contains("Expected three command angle words"), "{error}");
    }
}
