//! QuakeWorld application host types.
//!
//! Port of `src/app/bootstrap/network/qw-types.ts`. The donor is async
//! (`Promise`); this sync port resolves every host call inline. Existing
//! types are reused: [`ActorCommand`](qa_net::common::commands::ActorCommand)
//! for `contracts/session`, [`QwUsercmd`](qa_net::qw::QwUsercmd) for
//! `QwUserCommand`, and [`QuakeWorldMessage`](qa_net::q1_net::QuakeWorldMessage)
//! for `network/q1/quakeworld`. `QwServerData` (a donor `Extract` over the
//! `server-data` variant) becomes an owned struct with a fallible extractor.

use qa_core::math::Vec3;
use qa_net::common::commands::ActorCommand;
use qa_net::q1_net::{QuakeWorldMessage, QwDownload, QwMoveVariables};
use qa_net::q1_wide::QwProfile;
use qa_net::qw::QwUsercmd;
use thiserror::Error;

/// QuakeWorld server data (donor `QwServerData`).
#[derive(Debug, Clone, PartialEq)]
pub struct QwServerData {
    /// Protocol profile.
    pub protocol: QwProfile,
    /// Server count.
    pub server_count: i64,
    /// Game directory.
    pub game_directory: String,
    /// Player slot.
    pub player_slot: u8,
    /// Spectator flag.
    pub spectator: bool,
    /// Level name.
    pub level: String,
    /// Movement variables.
    pub move_variables: QwMoveVariables,
}

/// Error for QuakeWorld application host types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum QwTypesError {
    /// A message that is not server data was supplied.
    #[error("QuakeWorld message is not server data")]
    NotServerData,
}

/// Extract owned server data from a message (donor `QwServerData` extract).
pub fn qw_server_data(message: &QuakeWorldMessage) -> Result<QwServerData, QwTypesError> {
    if let QuakeWorldMessage::ServerData {
        protocol,
        server_count,
        game_directory,
        player_slot,
        spectator,
        level,
        move_variables,
    } = message
    {
        Ok(QwServerData {
            protocol: *protocol,
            server_count: *server_count,
            game_directory: game_directory.clone(),
            player_slot: *player_slot,
            spectator: *spectator,
            level: level.clone(),
            move_variables: move_variables.clone(),
        })
    } else {
        Err(QwTypesError::NotServerData)
    }
}

/// Download category (`'sound' | 'model' | 'skin'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwDownloadCategory {
    /// Sound download.
    Sound,
    /// Model download.
    Model,
    /// Skin download.
    Skin,
}

/// Download request verdict (`'available' | 'waiting' | 'skipped'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwDownloadRequest {
    /// Already available.
    Available,
    /// Waiting on the download.
    Waiting,
    /// Skipped.
    Skipped,
}

/// Download chunk verdict (`'waiting' | 'complete' | 'missing'`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QwDownloadReceive {
    /// More chunks are coming.
    Waiting,
    /// Download is complete.
    Complete,
    /// Download is missing.
    Missing,
}

/// QuakeWorld application downloads (donor `QwApplicationDownloads`).
pub trait QwApplicationDownloads {
    /// Request a download.
    fn request(&mut self, path: &str, category: QwDownloadCategory) -> QwDownloadRequest;
    /// Receive a download chunk.
    fn receive(&mut self, result: &QwDownload) -> QwDownloadReceive;
    /// Close downloads.
    fn close(&mut self);
}

/// QuakeWorld application skins (donor `QwApplicationClientHost['skins']`).
pub trait QwApplicationSkins {
    /// Skin names.
    fn names(&self) -> Vec<String>;
    /// Mark skins loading or idle.
    fn loading(&mut self, value: bool);
    /// Prepare skins.
    fn prepare(&mut self);
}

/// QuakeWorld application prediction (donor
/// `QwApplicationClientHost['prediction']`).
pub trait QwApplicationPrediction {
    /// Record a sent command.
    fn sent(&mut self, sequence: u32, command: &QwUsercmd, now_ms: f64);
    /// Record an acknowledged command.
    fn acknowledged(&mut self, sequence: u32, now_ms: f64);
}

/// QuakeWorld application client host (donor `QwApplicationClientHost`).
pub trait QwApplicationClientHost {
    /// Optional downloads.
    fn downloads(&mut self) -> Option<&mut dyn QwApplicationDownloads> {
        None
    }

    /// Optional skins.
    fn skins(&mut self) -> Option<&mut dyn QwApplicationSkins> {
        None
    }

    /// Optional prediction.
    fn prediction(&mut self) -> Option<&mut dyn QwApplicationPrediction> {
        None
    }

    /// Receive server data (donor `serverData`).
    fn server_data(&mut self, data: &QwServerData);

    /// Receive game state; returns the map checksum (donor `gameState`).
    fn game_state(&mut self, data: &QwServerData, models: &[String], sounds: &[String]) -> i32;

    /// Receive messages (donor `receive`).
    fn receive(&mut self, messages: &[QuakeWorldMessage], now_ms: f64);

    /// Convert an actor command (donor `command`).
    fn command(&mut self, command: &ActorCommand) -> QwUsercmd;

    /// Take a spectator teleport (donor `takeSpectatorTeleport?`).
    fn take_spectator_teleport(&mut self) -> Option<Vec3> {
        None
    }

    /// Handle a disconnect (donor `disconnected`).
    fn disconnected(&mut self, reason: &str);

    /// Print text (donor `print`).
    fn print(&mut self, text: &str);
}

#[cfg(test)]
mod tests {
    use super::*;
    use qa_core::identity::IdentityOwner;
    use qa_net::common::commands::{CommandSource, UserCommand};

    fn server_message() -> QuakeWorldMessage {
        QuakeWorldMessage::ServerData {
            protocol: QwProfile::Quakeworld,
            server_count: 3,
            game_directory: "qw".to_string(),
            player_slot: 1,
            spectator: false,
            level: "dm1".to_string(),
            move_variables: QwMoveVariables::default(),
        }
    }

    struct MockHost {
        checksums: Vec<i32>,
        log: Vec<String>,
    }

    impl QwApplicationClientHost for MockHost {
        fn server_data(&mut self, data: &QwServerData) {
            self.log.push(format!("server:{}", data.level));
        }

        fn game_state(
            &mut self,
            data: &QwServerData,
            models: &[String],
            sounds: &[String],
        ) -> i32 {
            let checksum = data.server_count as i32 + models.len() as i32 + sounds.len() as i32;
            self.checksums.push(checksum);
            checksum
        }

        fn receive(&mut self, messages: &[QuakeWorldMessage], now_ms: f64) {
            self.log.push(format!("receive:{}:{now_ms}", messages.len()));
        }

        fn command(&mut self, command: &ActorCommand) -> QwUsercmd {
            QwUsercmd {
                msec: command.sequence as u8,
                ..QwUsercmd::default()
            }
        }

        fn disconnected(&mut self, reason: &str) {
            self.log.push(format!("disconnect:{reason}"));
        }

        fn print(&mut self, text: &str) {
            self.log.push(text.to_string());
        }
    }

    #[test]
    fn server_data_extracts_owned_fields() {
        let data = qw_server_data(&server_message()).unwrap();
        assert_eq!(data.protocol, QwProfile::Quakeworld);
        assert_eq!(data.server_count, 3);
        assert_eq!(data.game_directory, "qw");
        assert_eq!(data.player_slot, 1);
        assert!(!data.spectator);
        assert_eq!(data.level, "dm1");
    }

    #[test]
    fn non_server_message_is_rejected() {
        let message = QuakeWorldMessage::Print {
            level: 0,
            text: "hi".to_string(),
        };
        assert_eq!(
            qw_server_data(&message),
            Err(QwTypesError::NotServerData)
        );
        assert_eq!(
            QwTypesError::NotServerData.to_string(),
            "QuakeWorld message is not server data"
        );
    }

    #[test]
    fn host_receives_lifecycle_calls() {
        let mut host = MockHost {
            checksums: Vec::new(),
            log: Vec::new(),
        };
        assert!(host.downloads().is_none());
        assert!(host.skins().is_none());
        assert!(host.prediction().is_none());
        assert!(host.take_spectator_teleport().is_none());

        let data = qw_server_data(&server_message()).unwrap();
        host.server_data(&data);
        let checksum = host.game_state(
            &data,
            &["progs/player.mdl".to_string()],
            &["weapons/shotgun.wav".to_string()],
        );
        assert_eq!(checksum, 5);
        host.receive(&[server_message()], 12.5);
        host.disconnected("kicked");
        host.print("done");
        assert_eq!(
            host.log,
            vec![
                "server:dm1".to_string(),
                "receive:1:12.5".to_string(),
                "disconnect:kicked".to_string(),
                "done".to_string(),
            ]
        );
    }

    #[test]
    fn host_converts_actor_commands() {
        let owner = IdentityOwner::create("qw-types-test").unwrap();
        let command = ActorCommand {
            actor: owner.actor(0, 1),
            source: CommandSource::LocalSeat {
                seat: owner.seat(0),
            },
            sequence: 300,
            command: UserCommand::Q1Quakeworld {
                milliseconds: 10.0,
                angles: [0.0, 0.0, 0.0],
                forward_move: 0.0,
                side_move: 0.0,
                up_move: 0.0,
                buttons: 0.0,
                impulse: 0.0,
            },
            arsenal: None,
        };
        let mut host = MockHost {
            checksums: Vec::new(),
            log: Vec::new(),
        };
        let converted = host.command(&command);
        assert_eq!(converted.msec, 44);
    }
}
