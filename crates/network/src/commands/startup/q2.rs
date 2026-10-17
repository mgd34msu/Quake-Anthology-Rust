use super::{Protocol, Reader, packet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ResourceStream {
    ConfigStrings,
    Baselines,
}

/// Protocol1038's native fields, independent of module and movement roles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q2ReproServerData {
    pub revision: u16,
    pub server_state: u8,
    pub flags: u16,
    pub server_fps: u8,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Q2ServerData<'a> {
    pub server_count: i32,
    pub attract_loop: bool,
    pub game_directory: &'a [u8],
    /// -1 denotes a cinematic or picture rather than a player assignment.
    pub client_number: i16,
    pub level_name: &'a [u8],
    pub rerelease: Option<Q2ReproServerData>,
}
impl<'a> Q2ServerData<'a> {
    pub fn cinematic(self) -> bool {
        self.client_number == -1
            || self
                .rerelease
                .is_some_and(|repro| matches!(repro.server_state, 3 | 5))
    }
    pub fn resource_layout(self) -> qa_core::configstrings::ConfigLayout {
        use qa_core::configstrings::ConfigLayout;
        match self.rerelease {
            Some(repro) if repro.flags & (1 << 15) != 0 => {
                ConfigLayout::quake2_compat(repro.flags & ((1 << 3) | (1 << 4)) != 0)
            }
            Some(_) => ConfigLayout::QUAKE2_RERELEASE,
            None => ConfigLayout::QUAKE2,
        }
    }
    pub(crate) fn read(protocol: Protocol, reader: &mut Reader<'a>) -> Result<Self, packet::Error> {
        if !matches!(protocol, Protocol::Quake2_34 | Protocol::Quake2Repro1038)
            || reader.read_bits(32)? != protocol.number()
        {
            return Err(packet::Error::Context);
        }
        let server_count = reader.read_signed(32)?;
        let attract_loop = reader.read_bits(8)? != 0;
        let game_directory = reader.read_string(protocol.string_policy())?;
        let client_number = reader.read_signed(16)? as i16;
        if client_number < -1
            || i32::from(client_number)
                >= crate::snapshots::snapshot_limits(protocol).client_capacity as i32
        {
            return Err(packet::Error::Count);
        }
        let level_name = reader.read_string(protocol.string_policy())?;
        let rerelease = if protocol == Protocol::Quake2Repro1038 {
            if game_directory.len() >= 64 {
                return Err(packet::Error::Count);
            }
            Some(Q2ReproServerData {
                revision: reader.read_bits(16)? as u16,
                server_state: reader.read_bits(8)? as u8,
                flags: reader.read_bits(16)? as u16,
                server_fps: reader.read_bits(8)? as u8,
            })
        } else {
            None
        };
        Ok(Self {
            server_count,
            attract_loop,
            game_directory,
            client_number,
            level_name,
            rerelease,
        })
    }

    /// Native Com_ComputeFrametime uses a clamped integer divider of 10Hz.
    pub fn frame_milliseconds(self) -> u16 {
        let Some(repro) = self.rerelease else {
            return 100;
        };
        let rate = u16::from(repro.server_fps);
        let rate = if self.attract_loop {
            (rate + 5) / 10 * 10
        } else {
            rate
        };
        100 / (rate / 10).clamp(1, 6)
    }
}
