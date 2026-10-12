//! Q2 module ABI limits are available independently of the host backend.
use crate::{memory::ModuleMemory, services::CallError};
use qa_core::primitives::{RuleSetId, UserCmd};

pub const INFO_KEY_BYTES: usize = 64;
pub const INFO_VALUE_BYTES: usize = 256;
/// bg_local.h CONFIG_N64_PHYSICS, checked against the original header enum.
pub const RERELEASE_N64_PHYSICS: usize = 12103;
#[derive(Clone, Copy)]
pub struct ClientLayout {
    pub userinfo_bytes: usize,
    pub value_bytes: usize,
    pub command_bytes: usize,
    write_command: fn(&UserCmd, u32, &mut [u8]),
    read_command: fn(&[u8], i32) -> UserCmd,
    pub(crate) stats: crate::players::StatsLayout,
}
pub const CLASSIC: ClientLayout = ClientLayout {
    userinfo_bytes: 512,
    value_bytes: INFO_KEY_BYTES,
    command_bytes: 16,
    write_command: classic_command,
    read_command: read_classic_command,
    stats: crate::players::StatsLayout {
        client: 88,
        offset: 120,
        protocol: qa_network::commands::packet::Protocol::Quake2_34,
    },
};
pub const RERELEASE: ClientLayout = ClientLayout {
    userinfo_bytes: 2048,
    value_bytes: INFO_VALUE_BYTES,
    command_bytes: 28,
    write_command: rerelease_command,
    read_command: read_rerelease_command,
    stats: crate::players::StatsLayout {
        client: 120,
        offset: 166,
        protocol: qa_network::commands::packet::Protocol::Quake2Repro1038,
    },
};
pub fn client_layout(rules: RuleSetId) -> Option<&'static ClientLayout> {
    match rules {
        RuleSetId::Quake2 => Some(&CLASSIC),
        RuleSetId::Quake2Rerelease => Some(&RERELEASE),
        _ => None,
    }
}
impl ClientLayout {
    pub fn read_command(
        &self,
        memory: &ModuleMemory<'_>,
        address: u64,
        server_time: i32,
    ) -> Result<UserCmd, CallError> {
        Ok((self.read_command)(
            memory.read(address, self.command_bytes)?,
            server_time,
        ))
    }
    /// This is game.h's in-memory usercmd_t, not a delta packet. The shared
    /// command projection owns all button, angle, movement and width rules.
    pub fn write_command(
        &self,
        memory: &mut ModuleMemory<'_>,
        address: u64,
        command: &UserCmd,
        server_frame: u32,
    ) -> Result<(), CallError> {
        let bytes = memory.read_mut(address, self.command_bytes)?;
        (self.write_command)(command, server_frame, bytes);
        Ok(())
    }
}
fn classic_command(command: &UserCmd, _: u32, bytes: &mut [u8]) {
    let native = qa_network::commands::to_q2_usercmd(command);
    bytes[0] = native.msec;
    bytes[1] = native.buttons;
    for (output, value) in bytes[2..14]
        .chunks_exact_mut(2)
        .zip(native.angles.into_iter().chain(native.movement))
    {
        output.copy_from_slice(&value.to_le_bytes());
    }
    bytes[14] = native.impulse;
    bytes[15] = native.light_level;
}
fn rerelease_command(command: &UserCmd, server_frame: u32, bytes: &mut [u8]) {
    let native = qa_network::commands::to_q2_rr_usercmd(command, server_frame);
    bytes[0] = native.msec;
    bytes[1] = native.buttons;
    bytes[2..4].fill(0);
    for (output, value) in bytes[4..24]
        .chunks_exact_mut(4)
        .zip(native.angles.0.into_iter().chain(native.movement))
    {
        output.copy_from_slice(&value.to_le_bytes());
    }
    bytes[24..28].copy_from_slice(&native.server_frame.to_le_bytes());
}

fn read_classic_command(bytes: &[u8], server_time: i32) -> UserCmd {
    let short = |offset| i16::from_le_bytes([bytes[offset], bytes[offset + 1]]);
    qa_network::commands::from_q2_usercmd(
        qa_network::commands::Q2Cmd {
            msec: bytes[0],
            buttons: bytes[1],
            angles: [short(2), short(4), short(6)],
            movement: [short(8), short(10), short(12)],
            impulse: bytes[14],
            light_level: bytes[15],
        },
        server_time,
    )
}
fn read_rerelease_command(bytes: &[u8], server_time: i32) -> UserCmd {
    let float = |offset| f32::from_le_bytes(std::array::from_fn(|i| bytes[offset + i]));
    qa_network::commands::from_q2_rr_usercmd(
        qa_network::commands::Q2RrCmd {
            msec: bytes[0],
            buttons: bytes[1],
            angles: qa_core::primitives::Vec3([float(4), float(8), float(12)]),
            movement: [float(16), float(20)],
            server_frame: u32::from_le_bytes(std::array::from_fn(|i| bytes[24 + i])),
        },
        server_time,
    )
}
