//! The connection owns its packet codec and fixed decode scratch.
use super::{
    from_q1_move, from_q2_usercmd, from_q3_usercmd, from_qw_usercmd,
    packet::{self, Key, Move, Protocol, ZERO_Q2, ZERO_Q3, ZERO_QW},
    to_q1_move, to_q2_usercmd, to_q3_usercmd, to_qw_usercmd,
};
use qa_core::primitives::UserCmd;

pub struct Commands {
    pub protocol: Protocol,
    scratch: Box<[u8]>,
    previous_time: i32,
}
impl Commands {
    pub fn load(protocol: Protocol) -> Self {
        Self {
            protocol,
            scratch: vec![0; 8192].into_boxed_slice(),
            previous_time: 0,
        }
    }
    /// Native packet redundancy is caller-supplied. The development host has
    /// one current command and no input history, so its older entries are zero.
    pub fn encode(
        &self,
        command: &UserCmd,
        sequence: u32,
        out: &mut [u8],
    ) -> Result<usize, packet::Error> {
        let movement = match self.protocol {
            Protocol::NetQuake15 => Move::NetQuake {
                timestamp: 0.,
                command: to_q1_move(command),
            },
            Protocol::QuakeWorld28 => Move::QuakeWorld {
                loss: 0,
                commands: [ZERO_QW, ZERO_QW, to_qw_usercmd(command)],
            },
            Protocol::Quake2_34 => Move::Quake2 {
                last_frame: -1,
                commands: [ZERO_Q2, ZERO_Q2, to_q2_usercmd(command)],
            },
            Protocol::Quake3_68 => {
                let mut commands = [ZERO_Q3; 32];
                // No registry/module binding exists in the development host.
                // Weapon zero is native NONE, never an ItemId/WeaponId cast.
                commands[0] = to_q3_usercmd(command, 0);
                Move::Quake3 {
                    commands,
                    count: 1,
                    delta: false,
                }
            }
        };
        packet::write(out, &movement, sequence, Key::default())
    }
    /// This development connection has no negotiated Q3 command history yet.
    /// Its explicit zero handshake context cannot accept arbitrary remote keys.
    /// NQ duration is the native SERVER frame input, never its ping timestamp.
    pub fn decode(
        &mut self,
        bytes: &[u8],
        sequence: u32,
        server_time: i32,
        frame_ns: u64,
    ) -> Result<Option<UserCmd>, packet::Error> {
        let scratch = self
            .scratch
            .get_mut(..bytes.len())
            .ok_or(packet::Error::Count)?;
        scratch.copy_from_slice(bytes);
        let movement = packet::read(self.protocol, scratch, sequence, Key::default())?;
        let command = match movement {
            Move::NetQuake { command, .. } => {
                let mut command = from_q1_move(
                    command,
                    (frame_ns / 1_000_000).min(u64::from(u16::MAX)) as u16,
                    server_time,
                );
                command.duration_ns = frame_ns;
                command
            }
            Move::QuakeWorld { commands, .. } => from_qw_usercmd(
                commands[2],
                self.previous_time.wrapping_add(i32::from(commands[2].msec)),
            ),
            Move::Quake2 { commands, .. } => from_q2_usercmd(
                commands[2],
                self.previous_time.wrapping_add(i32::from(commands[2].msec)),
            ),
            Move::Quake3 {
                commands, count, ..
            } => {
                let native = commands[usize::from(count) - 1];
                if native.server_time <= self.previous_time {
                    return Ok(None);
                }
                from_q3_usercmd(
                    native,
                    self.previous_time,
                    qa_core::primitives::WeaponId::default(),
                )
            }
        };
        self.previous_time = command.server_time_ms;
        Ok(Some(command))
    }
}
