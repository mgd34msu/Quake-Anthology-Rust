//! Q2 module ABI limits are available independently of the host backend.
use crate::{memory::ModuleMemory, services::CallError};
use qa_core::primitives::{
    ModelRef, ModuleId, NativePlayerTail, NativeViewFlags, PlayerState, ResourceNamespace,
    RuleSetId, UserCmd, ViewFlagLayout, ViewWeapon,
};

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
    presentation: PresentationLayout,
}
#[derive(Clone, Copy)]
struct PresentationLayout {
    view: usize,
    offset: usize,
    punch: usize,
    gun_angles: usize,
    gun_offset: usize,
    gun_model: usize,
    gun_frame: usize,
    gun_skin: Option<usize>,
    gun_rate: Option<usize>,
    view_height: Option<usize>,
    screen_blend: usize,
    damage_blend: Option<usize>,
    fov: usize,
    flags: usize,
    flags_width: crate::memory::Width,
    flags_layout: ViewFlagLayout,
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
    presentation: PresentationLayout {
        view: 28,
        offset: 40,
        punch: 52,
        gun_angles: 64,
        gun_offset: 76,
        gun_model: 88,
        gun_frame: 92,
        gun_skin: None,
        gun_rate: None,
        view_height: None,
        screen_blend: 96,
        damage_blend: None,
        fov: 112,
        flags: 116,
        flags_width: crate::memory::Width::Word,
        flags_layout: ViewFlagLayout::CLASSIC,
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
    presentation: PresentationLayout {
        view: 52,
        offset: 64,
        punch: 76,
        gun_angles: 88,
        gun_offset: 100,
        gun_model: 112,
        gun_frame: 120,
        gun_skin: Some(116),
        gun_rate: Some(124),
        view_height: Some(48),
        screen_blend: 128,
        damage_blend: Some(144),
        fov: 160,
        flags: 164,
        flags_width: crate::memory::Width::Byte,
        flags_layout: ViewFlagLayout::KEX,
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
    /// Shared native player-state presentation, independent of its selected
    /// movement rules. These offsets are offsetof(player_state_t, ...) from
    /// the original classic/rerelease game headers, not packet-word offsets.
    pub(crate) fn import_presentation(
        &self,
        memory: &ModuleMemory<'_>,
        pointer: u64,
        module: ModuleId,
        player: &mut PlayerState,
    ) -> Result<(), CallError> {
        let layout = self.presentation;
        player.view_angles = memory.read_vec3(pointer + layout.view as u64)?;
        let offset = memory.read_vec3(pointer + layout.offset as u64)?;
        let height = layout
            .view_height
            .map(|height| {
                memory
                    .read_field(pointer + height as u64, crate::memory::Width::Byte)
                    .map(|height| f32::from(height as i8))
            })
            .transpose()?;
        player.view.accept_offset(offset, height);
        player.movement.view_height = height.unwrap_or(offset.0[2]);
        player.punch_angles = memory.read_vec3(pointer + layout.punch as u64)?;
        player.view_weapon = ViewWeapon {
            model: ModelRef::native(
                ResourceNamespace::Module(module),
                memory.read_word(pointer + layout.gun_model as u64)? as u32,
            ),
            frame: memory.read_word(pointer + layout.gun_frame as u64)?,
            offset: memory.read_vec3(pointer + layout.gun_offset as u64)?,
            angles: memory.read_vec3(pointer + layout.gun_angles as u64)?,
            skin: layout
                .gun_skin
                .map(|at| memory.read_word(pointer + at as u64))
                .transpose()?
                .unwrap_or(0),
            rate: layout
                .gun_rate
                .map(|at| memory.read_word(pointer + at as u64))
                .transpose()?
                .unwrap_or(0),
        };
        let read_float = |at| -> Result<f32, CallError> {
            let value =
                f32::from_bits(memory.read_field(pointer + at as u64, crate::memory::Width::Word)?);
            value.is_finite().then_some(value).ok_or(CallError::Memory)
        };
        let read_color = |at| -> Result<[f32; 4], CallError> {
            Ok([
                read_float(at)?,
                read_float(at + 4)?,
                read_float(at + 8)?,
                read_float(at + 12)?,
            ])
        };
        let screen_blend = read_color(layout.screen_blend)?;
        let damage_blend = layout
            .damage_blend
            .map(read_color)
            .transpose()?
            .unwrap_or_default();
        let fov = read_float(layout.fov)?;
        let raw = memory.read_field(pointer + layout.flags as u64, layout.flags_width)?;
        player.view.screen_blend = screen_blend;
        player.view.damage_blend = damage_blend;
        player.view.fov_x = Some(fov);
        player.view.flags = layout.flags_layout.decode(raw);
        let source = self.stats.protocol.rule_set();
        let namespace = ResourceNamespace::Module(module);
        if player.native_for(source, Some(namespace)).is_none() {
            player.native = Some(NativePlayerTail::new(source, namespace));
        }
        if let Some(native) = &mut player.native {
            native.view_flags = Some(NativeViewFlags {
                layout: layout.flags_layout,
                raw,
            });
        }
        Ok(())
    }
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

#[cfg(test)]
#[path = "../tests/unit/q2_presentation.rs"]
mod tests;
