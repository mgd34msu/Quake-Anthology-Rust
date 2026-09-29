//! Wire identities, ports, limits, opcodes, and update bits ported from
//! `src/contracts/protocol.ts`, `src/network/q1/constants.ts`,
//! `src/network/q1/qw-constants.ts`, `src/network/q2/constants.ts`, and
//! `src/network/q3/adapters.ts` (`Q3_PROTOCOL`).
//!
//! Wire versions select codecs. They do not select game APIs or frame cadence.

/// Protocol wire identity.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ProtocolIdentity {
    /// NetQuake protocol 15.
    Q1Netquake,
    /// FitzQuake protocol 666.
    Q1Fitzquake,
    /// RMQ protocol 999 with `PRFL_` flags.
    Q1Rmq {
        /// Protocol flags (`PRFL_*`).
        flags: u32,
    },
    /// QuakeWorld protocol 28.
    Q1Quakeworld,
    /// Donor wide-protocol 29 with flags.
    Q1QuakeworldWide {
        /// Protocol flags.
        flags: u32,
    },
    /// Classic Quake II protocol 34.
    Q2Classic,
    /// R1Q2 protocol 35 with revision.
    Q2R1q2 {
        /// Revision: 1903, 1904, or 1905.
        revision: u32,
    },
    /// Q2Pro protocol 36 with revision.
    Q2Q2pro {
        /// Revision: 1015 through 1026.
        revision: u32,
    },
    /// Rerelease protocol 1038.
    Q2Rerelease,
    /// Kex protocol 2023.
    Q2Kex,
    /// Kex demo protocol 2022.
    Q2KexDemo,
    /// Private classic-compatible protocol 4038.
    Q2PrivateClassic,
    /// Quake III protocol 68.
    Q3,
}

impl ProtocolIdentity {
    /// Numeric wire version.
    #[must_use]
    pub fn version(&self) -> u32 {
        match *self {
            ProtocolIdentity::Q1Netquake => q1::PROTOCOL_VERSION,
            ProtocolIdentity::Q1Fitzquake => q1::PROTOCOL_FITZQUAKE,
            ProtocolIdentity::Q1Rmq { .. } => q1::PROTOCOL_RMQ,
            ProtocolIdentity::Q1Quakeworld => qw::PROTOCOL_VERSION,
            ProtocolIdentity::Q1QuakeworldWide { .. } => 29,
            ProtocolIdentity::Q2Classic => q2::PROTOCOL_VERSION,
            ProtocolIdentity::Q2R1q2 { .. } => q2::PROTOCOL_VERSION_R1Q2,
            ProtocolIdentity::Q2Q2pro { .. } => q2::PROTOCOL_VERSION_Q2PRO,
            ProtocolIdentity::Q2Rerelease => q2::PROTOCOL_VERSION_RERELEASE,
            ProtocolIdentity::Q2Kex => 2023,
            ProtocolIdentity::Q2KexDemo => 2022,
            ProtocolIdentity::Q2PrivateClassic => q2::PROTOCOL_VERSION_RERELEASE_CLASSIC,
            ProtocolIdentity::Q3 => q3::PROTOCOL_VERSION,
        }
    }
}

/// NetQuake / FitzQuake / RMQ constants.
pub mod q1 {
    /// NetQuake wire version.
    pub const PROTOCOL_VERSION: u32 = 15;
    /// NetQuake wire version alias.
    pub const PROTOCOL_NETQUAKE: u32 = 15;
    /// FitzQuake wire version.
    pub const PROTOCOL_FITZQUAKE: u32 = 666;
    /// RMQ wire version.
    pub const PROTOCOL_RMQ: u32 = 999;

    /// NetQuake maximum message length.
    pub const NQ15_MAX_MSGLEN: usize = 8000;
    /// Wide-protocol maximum message length.
    pub const WIDE_MAX_MSGLEN: usize = 64000;

    /// Short angles instead of bytes.
    pub const PRFL_SHORTANGLE: u32 = 1 << 1;
    /// Float angles.
    pub const PRFL_FLOATANGLE: u32 = 1 << 2;
    /// 24-bit coordinates.
    pub const PRFL_24BITCOORD: u32 = 1 << 3;
    /// Float coordinates.
    pub const PRFL_FLOATCOORD: u32 = 1 << 4;
    /// Scaled entity byte.
    pub const PRFL_EDICTSCALE: u32 = 1 << 5;
    /// Alpha sanity cleanup.
    pub const PRFL_ALPHASANITY: u32 = 1 << 6;
    /// 32-bit integer coordinates.
    pub const PRFL_INT32COORD: u32 = 1 << 7;
    /// More flags follow (unsupported).
    pub const PRFL_MOREFLAGS: u32 = 1 << 31;
    /// Flags this implementation supports.
    pub const PRFL_SUPPORTED: u32 =
        PRFL_SHORTANGLE | PRFL_FLOATANGLE | PRFL_24BITCOORD | PRFL_FLOATCOORD | PRFL_EDICTSCALE | PRFL_INT32COORD;

    /// Update bits: more bits follow.
    pub const U_MOREBITS: u32 = 1 << 0;
    /// Update bits: origin x.
    pub const U_ORIGIN1: u32 = 1 << 1;
    /// Update bits: origin y.
    pub const U_ORIGIN2: u32 = 1 << 2;
    /// Update bits: origin z.
    pub const U_ORIGIN3: u32 = 1 << 3;
    /// Update bits: yaw.
    pub const U_ANGLE2: u32 = 1 << 4;
    /// Update bits: no interpolation.
    pub const U_NOLERP: u32 = 1 << 5;
    /// Update bits: step (alias for no-lerp).
    pub const U_STEP: u32 = U_NOLERP;
    /// Update bits: frame.
    pub const U_FRAME: u32 = 1 << 6;
    /// Update bits: signal marker.
    pub const U_SIGNAL: u32 = 1 << 7;
    /// Update bits: pitch.
    pub const U_ANGLE1: u32 = 1 << 8;
    /// Update bits: roll.
    pub const U_ANGLE3: u32 = 1 << 9;
    /// Update bits: model index.
    pub const U_MODEL: u32 = 1 << 10;
    /// Update bits: colormap.
    pub const U_COLORMAP: u32 = 1 << 11;
    /// Update bits: skin.
    pub const U_SKIN: u32 = 1 << 12;
    /// Update bits: effects.
    pub const U_EFFECTS: u32 = 1 << 13;
    /// Update bits: long entity number.
    pub const U_LONGENTITY: u32 = 1 << 14;
    /// Update bits: extension byte follows.
    pub const U_EXTEND1: u32 = 1 << 15;
    /// Update bits: alpha byte.
    pub const U_ALPHA: u32 = 1 << 16;
    /// Update bits: frame high byte.
    pub const U_FRAME2: u32 = 1 << 17;
    /// Update bits: model high byte.
    pub const U_MODEL2: u32 = 1 << 18;
    /// Update bits: lerp finish byte.
    pub const U_LERPFINISH: u32 = 1 << 19;
    /// Update bits: scale byte.
    pub const U_SCALE: u32 = 1 << 20;
    /// Update bits: second extension byte follows.
    pub const U_EXTEND2: u32 = 1 << 23;

    /// Client-data bits: view height.
    pub const SU_VIEWHEIGHT: u32 = 1 << 0;
    /// Client-data bits: ideal pitch.
    pub const SU_IDEALPITCH: u32 = 1 << 1;
    /// Client-data bits: punch pitch.
    pub const SU_PUNCH1: u32 = 1 << 2;
    /// Client-data bits: punch yaw.
    pub const SU_PUNCH2: u32 = 1 << 3;
    /// Client-data bits: punch roll.
    pub const SU_PUNCH3: u32 = 1 << 4;
    /// Client-data bits: velocity x.
    pub const SU_VELOCITY1: u32 = 1 << 5;
    /// Client-data bits: velocity y.
    pub const SU_VELOCITY2: u32 = 1 << 6;
    /// Client-data bits: velocity z.
    pub const SU_VELOCITY3: u32 = 1 << 7;
    /// Client-data bits: items.
    pub const SU_ITEMS: u32 = 1 << 9;
    /// Client-data bits: on ground (no data follows).
    pub const SU_ONGROUND: u32 = 1 << 10;
    /// Client-data bits: in water (no data follows).
    pub const SU_INWATER: u32 = 1 << 11;
    /// Client-data bits: weapon frame.
    pub const SU_WEAPONFRAME: u32 = 1 << 12;
    /// Client-data bits: armor.
    pub const SU_ARMOR: u32 = 1 << 13;
    /// Client-data bits: weapon model.
    pub const SU_WEAPON: u32 = 1 << 14;
    /// Client-data bits: extension byte follows.
    pub const SU_EXTEND1: u32 = 1 << 15;
    /// Client-data bits: weapon high byte.
    pub const SU_WEAPON2: u32 = 1 << 16;
    /// Client-data bits: armor high byte.
    pub const SU_ARMOR2: u32 = 1 << 17;
    /// Client-data bits: ammo high byte.
    pub const SU_AMMO2: u32 = 1 << 18;
    /// Client-data bits: shells high byte.
    pub const SU_SHELLS2: u32 = 1 << 19;
    /// Client-data bits: nails high byte.
    pub const SU_NAILS2: u32 = 1 << 20;
    /// Client-data bits: rockets high byte.
    pub const SU_ROCKETS2: u32 = 1 << 21;
    /// Client-data bits: cells high byte.
    pub const SU_CELLS2: u32 = 1 << 22;
    /// Client-data bits: second extension follows.
    pub const SU_EXTEND2: u32 = 1 << 23;
    /// Client-data bits: weapon frame high byte.
    pub const SU_WEAPONFRAME2: u32 = 1 << 24;
    /// Client-data bits: weapon alpha.
    pub const SU_WEAPONALPHA: u32 = 1 << 25;
    /// Client-data bits: third extension follows.
    pub const SU_EXTEND3: u32 = 1 << 31;

    /// Sound bits: volume byte follows.
    pub const SND_VOLUME: u32 = 1 << 0;
    /// Sound bits: attenuation byte follows.
    pub const SND_ATTENUATION: u32 = 1 << 1;
    /// Sound bits: looping sound.
    pub const SND_LOOPING: u32 = 1 << 2;
    /// Sound bits: long entity + channel.
    pub const SND_LARGEENTITY: u32 = 1 << 3;
    /// Sound bits: short sound index.
    pub const SND_LARGESOUND: u32 = 1 << 4;

    /// Baseline bits: short model index.
    pub const B_LARGEMODEL: u32 = 1 << 0;
    /// Baseline bits: short frame.
    pub const B_LARGEFRAME: u32 = 1 << 1;
    /// Baseline bits: alpha byte.
    pub const B_ALPHA: u32 = 1 << 2;
    /// Baseline bits: scale byte.
    pub const B_SCALE: u32 = 1 << 3;

    /// Default view height.
    pub const DEFAULT_VIEWHEIGHT: i32 = 22;
    /// Default sound packet volume.
    pub const DEFAULT_SOUND_PACKET_VOLUME: i32 = 255;
    /// Default sound packet attenuation.
    pub const DEFAULT_SOUND_PACKET_ATTENUATION: f64 = 1.0;

    /// Entity alpha: default (water obeys `r_wateralpha`).
    pub const ENTALPHA_DEFAULT: u8 = 0;
    /// Entity alpha: invisible.
    pub const ENTALPHA_ZERO: u8 = 1;
    /// Entity alpha: fully opaque.
    pub const ENTALPHA_ONE: u8 = 255;
    /// Entity scale byte for 1.0.
    pub const ENTSCALE_DEFAULT: u8 = 16;

    /// Server-to-client opcodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum Svc {
        /// Invalid opcode.
        Bad = 0,
        /// No operation.
        Nop = 1,
        /// Disconnect.
        Disconnect = 2,
        /// Update a stat.
        Updatestat = 3,
        /// Server version.
        Version = 4,
        /// Set view entity.
        Setview = 5,
        /// Start a sound.
        Sound = 6,
        /// Server time.
        Time = 7,
        /// Print text.
        Print = 8,
        /// Stuff console text.
        Stufftext = 9,
        /// Set view angles.
        Setangle = 10,
        /// Server info.
        Serverinfo = 11,
        /// Light style.
        Lightstyle = 12,
        /// Update client name.
        Updatename = 13,
        /// Update frags.
        Updatefrags = 14,
        /// Client data.
        Clientdata = 15,
        /// Stop a sound.
        Stopsound = 16,
        /// Update colors.
        Updatecolors = 17,
        /// Particle.
        Particle = 18,
        /// Damage feedback.
        Damage = 19,
        /// Spawn static entity.
        Spawnstatic = 20,
        /// Spawn baseline.
        Spawnbaseline = 22,
        /// Temporary entity.
        TempEntity = 23,
        /// Set pause.
        Setpause = 24,
        /// Signon sequence number.
        Signonnum = 25,
        /// Center print.
        Centerprint = 26,
        /// Monster killed.
        Killedmonster = 27,
        /// Secret found.
        Foundsecret = 28,
        /// Static sound.
        Spawnstaticsound = 29,
        /// Intermission.
        Intermission = 30,
        /// Finale.
        Finale = 31,
        /// CD track.
        Cdtrack = 32,
        /// Sell screen.
        Sellscreen = 33,
        /// Cutscene.
        Cutscene = 34,
    }

    /// Client-to-server opcodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum Clc {
        /// Invalid opcode.
        Bad = 0,
        /// No operation.
        Nop = 1,
        /// Disconnect.
        Disconnect = 2,
        /// Move command.
        Move = 3,
        /// String command.
        Stringcmd = 4,
    }

    /// Temporary entity types.
    pub const TE_SPIKE: u8 = 0;
    /// Super spike.
    pub const TE_SUPERSPIKE: u8 = 1;
    /// Gunshot.
    pub const TE_GUNSHOT: u8 = 2;
    /// Explosion.
    pub const TE_EXPLOSION: u8 = 3;
    /// Tar explosion.
    pub const TE_TAREXPLOSION: u8 = 4;
    /// Lightning bolt 1.
    pub const TE_LIGHTNING1: u8 = 5;
    /// Lightning bolt 2.
    pub const TE_LIGHTNING2: u8 = 6;
    /// Wizard spike.
    pub const TE_WIZSPIKE: u8 = 7;
    /// Knight spike.
    pub const TE_KNIGHTSPIKE: u8 = 8;
    /// Lightning bolt 3.
    pub const TE_LIGHTNING3: u8 = 9;
    /// Lava splash.
    pub const TE_LAVASPLASH: u8 = 10;
    /// Teleport splash.
    pub const TE_TELEPORT: u8 = 11;
    /// Second explosion style.
    pub const TE_EXPLOSION2: u8 = 12;
    /// Beam.
    pub const TE_BEAM: u8 = 13;

    /// Extended server opcodes (FitzQuake/RMQ/re-release).
    pub const SVC_SKYBOX: u8 = 37;
    /// Bot chat line.
    pub const SVC_BOTCHAT: u8 = 38;
    /// Spawned monster count.
    pub const SVC_SPAWNEDMONSTER: u8 = 39;
    /// Extended messages.
    pub const SVC_BF: u8 = 40;
    /// Fog parameters.
    pub const SVC_FOG: u8 = 41;
    /// Baseline with flags.
    pub const SVC_SPAWNBASELINE2: u8 = 42;
    /// Static entity with flags.
    pub const SVC_SPAWNSTATIC2: u8 = 43;
    /// Static sound with short index.
    pub const SVC_SPAWNSTATICSOUND2: u8 = 44;
    /// Splitscreen seat count.
    pub const SVC_SETVIEWS: u8 = 45;
    /// Client ping update.
    pub const SVC_UPDATEPING: u8 = 46;
    /// Client social id update.
    pub const SVC_UPDATESOCIAL: u8 = 47;
    /// Client info update.
    pub const SVC_UPDATEPLINFO: u8 = 48;
    /// Raw print.
    pub const SVC_RAWPRINT: u8 = 49;
    /// Server variables.
    pub const SVC_SERVERVARS: u8 = 50;
    /// Sequence number.
    pub const SVC_SEQ: u8 = 51;
    /// Achievement.
    pub const SVC_ACHIEVEMENT: u8 = 52;
    /// Chat line.
    pub const SVC_CHAT: u8 = 53;
    /// Level completed.
    pub const SVC_LEVELCOMPLETED: u8 = 54;
    /// Back to lobby.
    pub const SVC_BACKTOLOBBY: u8 = 55;
    /// Local sound.
    pub const SVC_LOCALSOUND: u8 = 56;
    /// Prompt operation.
    pub const SVC_PROMPT: u8 = 57;

    /// Prompt begin operation.
    pub const PROMPT_BEGIN: u8 = 0;
    /// Prompt choice operation.
    pub const PROMPT_CHOICE: u8 = 1;
    /// Prompt clear operation.
    pub const PROMPT_CLEAR: u8 = 2;
}

/// QuakeWorld constants.
pub mod qw {
    /// QuakeWorld wire version.
    pub const PROTOCOL_VERSION: u32 = 28;
    /// Donor wide-protocol wire version.
    pub const PROTOCOL_VERSION_WIDE: u32 = 29;
    /// Challenge hash marker.
    pub const QW_CHECK_HASH: u32 = 0x5157;
    /// Client port.
    pub const PORT_CLIENT: u16 = 27001;
    /// Master port.
    pub const PORT_MASTER: u16 = 27000;
    /// Server port.
    pub const PORT_SERVER: u16 = 27500;
    /// Maximum message length.
    pub const MAX_MSGLEN: usize = 1450;
    /// Maximum clients.
    pub const MAX_CLIENTS: usize = 32;
    /// Buffered entity-state copies.
    pub const UPDATE_BACKUP: usize = 64;
    /// Entity-state buffer mask.
    pub const UPDATE_MASK: usize = UPDATE_BACKUP - 1;
    /// Packet entities per message (excluding nails).
    pub const MAX_PACKET_ENTITIES: usize = 64;

    /// Playerinfo bits: milliseconds.
    pub const PF_MSEC: u32 = 1 << 0;
    /// Playerinfo bits: command.
    pub const PF_COMMAND: u32 = 1 << 1;
    /// Playerinfo bits: velocity x.
    pub const PF_VELOCITY1: u32 = 1 << 2;
    /// Playerinfo bits: velocity y.
    pub const PF_VELOCITY2: u32 = 1 << 3;
    /// Playerinfo bits: velocity z.
    pub const PF_VELOCITY3: u32 = 1 << 4;
    /// Playerinfo bits: model.
    pub const PF_MODEL: u32 = 1 << 5;
    /// Playerinfo bits: skin.
    pub const PF_SKINNUM: u32 = 1 << 6;
    /// Playerinfo bits: effects.
    pub const PF_EFFECTS: u32 = 1 << 7;
    /// Playerinfo bits: weapon frame (view player only).
    pub const PF_WEAPONFRAME: u32 = 1 << 8;
    /// Playerinfo bits: dead (no movement blocking).
    pub const PF_DEAD: u32 = 1 << 9;
    /// Playerinfo bits: gibbed (offset view height).
    pub const PF_GIB: u32 = 1 << 10;
    /// Playerinfo bits: no gravity in prediction.
    pub const PF_NOGRAV: u32 = 1 << 11;

    /// User-command bits: pitch.
    pub const CM_ANGLE1: u32 = 1 << 0;
    /// User-command bits: roll.
    pub const CM_ANGLE3: u32 = 1 << 1;
    /// User-command bits: forward move.
    pub const CM_FORWARD: u32 = 1 << 2;
    /// User-command bits: side move.
    pub const CM_SIDE: u32 = 1 << 3;
    /// User-command bits: up move.
    pub const CM_UP: u32 = 1 << 4;
    /// User-command bits: buttons.
    pub const CM_BUTTONS: u32 = 1 << 5;
    /// User-command bits: impulse.
    pub const CM_IMPULSE: u32 = 1 << 6;
    /// User-command bits: yaw.
    pub const CM_ANGLE2: u32 = 1 << 7;

    /// Update bits: pitch.
    pub const U_ANGLE1: u32 = 1 << 0;
    /// Update bits: roll.
    pub const U_ANGLE3: u32 = 1 << 1;
    /// Update bits: model.
    pub const U_MODEL: u32 = 1 << 2;
    /// Update bits: colormap.
    pub const U_COLORMAP: u32 = 1 << 3;
    /// Update bits: skin.
    pub const U_SKIN: u32 = 1 << 4;
    /// Update bits: effects.
    pub const U_EFFECTS: u32 = 1 << 5;
    /// Update bits: solid for prediction.
    pub const U_SOLID: u32 = 1 << 6;
    /// Update bits: origin x.
    pub const U_ORIGIN1: u32 = 1 << 9;
    /// Update bits: origin y.
    pub const U_ORIGIN2: u32 = 1 << 10;
    /// Update bits: origin z.
    pub const U_ORIGIN3: u32 = 1 << 11;
    /// Update bits: yaw.
    pub const U_ANGLE2: u32 = 1 << 12;
    /// Update bits: frame.
    pub const U_FRAME: u32 = 1 << 13;
    /// Update bits: remove this entity.
    pub const U_REMOVE: u32 = 1 << 14;
    /// Update bits: more bits follow.
    pub const U_MOREBITS: u32 = 1 << 15;

    /// Sound bits: volume byte.
    pub const SND_VOLUME: u32 = 1 << 15;
    /// Sound bits: attenuation byte.
    pub const SND_ATTENUATION: u32 = 1 << 14;

    /// Print channel: low.
    pub const PRINT_LOW: u8 = 0;
    /// Print channel: medium.
    pub const PRINT_MEDIUM: u8 = 1;
    /// Print channel: high.
    pub const PRINT_HIGH: u8 = 2;
    /// Print channel: chat.
    pub const PRINT_CHAT: u8 = 3;

    /// Server-to-client opcodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum Svc {
        /// Invalid opcode.
        Bad = 0,
        /// No operation.
        Nop = 1,
        /// Disconnect.
        Disconnect = 2,
        /// Update a stat.
        Updatestat = 3,
        /// Set view entity.
        Setview = 5,
        /// Start a sound.
        Sound = 6,
        /// Print text.
        Print = 8,
        /// Stuff console text.
        Stufftext = 9,
        /// Set view angles.
        Setangle = 10,
        /// Server data.
        Serverdata = 11,
        /// Light style.
        Lightstyle = 12,
        /// Update frags.
        Updatefrags = 14,
        /// Stop a sound.
        Stopsound = 16,
        /// Damage feedback.
        Damage = 19,
        /// Spawn static entity.
        Spawnstatic = 20,
        /// Spawn baseline.
        Spawnbaseline = 22,
        /// Temporary entity.
        TempEntity = 23,
        /// Set pause.
        Setpause = 24,
        /// Center print.
        Centerprint = 26,
        /// Monster killed.
        Killedmonster = 27,
        /// Secret found.
        Foundsecret = 28,
        /// Static sound.
        Spawnstaticsound = 29,
        /// Intermission.
        Intermission = 30,
        /// Finale.
        Finale = 31,
        /// CD track.
        Cdtrack = 32,
        /// Sell screen.
        Sellscreen = 33,
        /// Small kick.
        Smallkick = 34,
        /// Big kick.
        Bigkick = 35,
        /// Ping update.
        Updateping = 36,
        /// Enter-time update.
        Updateentertime = 37,
        /// Long stat update.
        Updatestatlong = 38,
        /// Muzzle flash.
        Muzzleflash = 39,
        /// Userinfo update.
        Updateuserinfo = 40,
        /// Download chunk.
        Download = 41,
        /// Player info.
        Playerinfo = 42,
        /// Nail update.
        Nails = 43,
        /// Choke count.
        Chokecount = 44,
        /// Model list.
        Modellist = 45,
        /// Sound list.
        Soundlist = 46,
        /// Packet entities.
        Packetentities = 47,
        /// Delta packet entities.
        Deltapacketentities = 48,
        /// Max speed change.
        Maxspeed = 49,
        /// Gravity change.
        Entgravity = 50,
        /// Client setinfo.
        Setinfo = 51,
        /// Server info.
        Serverinfo = 52,
        /// Packet loss update.
        Updatepl = 53,
    }

    /// Client-to-server opcodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum Clc {
        /// Invalid opcode.
        Bad = 0,
        /// No operation.
        Nop = 1,
        /// Move command.
        Move = 3,
        /// String command.
        Stringcmd = 4,
        /// Request delta compression.
        Delta = 5,
        /// Teleport request (spectator).
        Tmove = 6,
        /// Upload chunk.
        Upload = 7,
    }
}

/// Quake II constants.
pub mod q2 {
    /// Classic wire version.
    pub const PROTOCOL_VERSION: u32 = 34;
    /// Rerelease wire version.
    pub const PROTOCOL_VERSION_RERELEASE: u32 = 1038;
    /// Private classic-compatible wire version.
    pub const PROTOCOL_VERSION_RERELEASE_CLASSIC: u32 = 4038;
    /// R1Q2 wire version.
    pub const PROTOCOL_VERSION_R1Q2: u32 = 35;
    /// Minimum R1Q2 revision.
    pub const PROTOCOL_VERSION_R1Q2_MINIMUM: u32 = 1903;
    /// R1Q2 user-command revision.
    pub const PROTOCOL_VERSION_R1Q2_UCMD: u32 = 1904;
    /// R1Q2 long-solid revision.
    pub const PROTOCOL_VERSION_R1Q2_LONG_SOLID: u32 = 1905;
    /// Current R1Q2 revision.
    pub const PROTOCOL_VERSION_R1Q2_CURRENT: u32 = 1905;
    /// Q2Pro wire version.
    pub const PROTOCOL_VERSION_Q2PRO: u32 = 36;
    /// Minimum Q2Pro revision.
    pub const PROTOCOL_VERSION_Q2PRO_MINIMUM: u32 = 1015;
    /// Q2Pro server-state revision.
    pub const PROTOCOL_VERSION_Q2PRO_SERVER_STATE: u32 = 1019;
    /// Current Q2Pro revision.
    pub const PROTOCOL_VERSION_Q2PRO_CURRENT: u32 = PROTOCOL_VERSION_Q2PRO_SERVER_STATE;

    /// Extra player-state bits: gun offset.
    pub const EPS_GUNOFFSET: u32 = 1 << 0;
    /// Extra player-state bits: gun angles.
    pub const EPS_GUNANGLES: u32 = 1 << 1;
    /// Extra player-state bits: mec velocity.
    pub const EPS_M_VELOCITY2: u32 = 1 << 2;
    /// Extra player-state bits: mec origin.
    pub const EPS_M_ORIGIN2: u32 = 1 << 3;
    /// Extra player-state bits: second view angle.
    pub const EPS_VIEWANGLE2: u32 = 1 << 4;
    /// Extra player-state bits: stats.
    pub const EPS_STATS: u32 = 1 << 5;

    /// Compressed packet opcode.
    pub const SVC_ZPACKET: u8 = 21;

    /// Master port.
    pub const PORT_MASTER: u16 = 27900;
    /// Client port.
    pub const PORT_CLIENT: u16 = 27901;
    /// Server port.
    pub const PORT_SERVER: u16 = 27910;
    /// Any port.
    pub const PORT_ANY: i32 = -1;

    /// Buffered frames.
    pub const UPDATE_BACKUP: usize = 16;
    /// Frame buffer mask.
    pub const UPDATE_MASK: usize = UPDATE_BACKUP - 1;
    /// Maximum message length.
    pub const MAX_MSGLEN: usize = 1400;
    /// Packet header size.
    pub const PACKET_HEADER: usize = 10;

    /// Fatal error.
    pub const ERR_FATAL: u8 = 0;
    /// Drop error.
    pub const ERR_DROP: u8 = 1;
    /// Quit error.
    pub const ERR_QUIT: u8 = 2;

    /// Server-to-client opcodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum Svc {
        /// Invalid opcode.
        Bad = 0,
        /// Muzzle flash.
        Muzzleflash = 1,
        /// Second muzzle flash style.
        Muzzleflash2 = 2,
        /// Temporary entity.
        TempEntity = 3,
        /// Layout.
        Layout = 4,
        /// Inventory.
        Inventory = 5,
        /// No operation.
        Nop = 6,
        /// Disconnect.
        Disconnect = 7,
        /// Reconnect.
        Reconnect = 8,
        /// Sound.
        Sound = 9,
        /// Print.
        Print = 10,
        /// Stuff text.
        Stufftext = 11,
        /// Server data.
        Serverdata = 12,
        /// Config string.
        Configstring = 13,
        /// Spawn baseline.
        Spawnbaseline = 14,
        /// Center print.
        Centerprint = 15,
        /// Download.
        Download = 16,
        /// Player info.
        Playerinfo = 17,
        /// Packet entities.
        Packetentities = 18,
        /// Delta packet entities.
        Deltapacketentities = 19,
        /// Frame.
        Frame = 20,
    }

    /// Client-to-server opcodes.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    #[repr(u8)]
    pub enum Clc {
        /// Invalid opcode.
        Bad = 0,
        /// No operation.
        Nop = 1,
        /// Move command.
        Move = 2,
        /// Userinfo.
        Userinfo = 3,
        /// String command.
        Stringcmd = 4,
        /// R1Q2 setting.
        R1q2Setting = 5,
        /// Q2Pro move without delta.
        Q2proMoveNodelta = 10,
        /// Q2Pro batched move.
        Q2proMoveBatched = 11,
        /// Q2Pro userinfo delta.
        Q2proUserinfoDelta = 12,
    }

    /// User-command bits: pitch.
    pub const CM_ANGLE1: u32 = 1 << 0;
    /// User-command bits: yaw.
    pub const CM_ANGLE2: u32 = 1 << 1;
    /// User-command bits: roll.
    pub const CM_ANGLE3: u32 = 1 << 2;
    /// User-command bits: forward move.
    pub const CM_FORWARD: u32 = 1 << 3;
    /// User-command bits: side move.
    pub const CM_SIDE: u32 = 1 << 4;
    /// User-command bits: up move.
    pub const CM_UP: u32 = 1 << 5;
    /// User-command bits: buttons.
    pub const CM_BUTTONS: u32 = 1 << 6;
    /// User-command bits: impulse.
    pub const CM_IMPULSE: u32 = 1 << 7;

    /// Entity bits: origin x.
    pub const U_ORIGIN1: u32 = 1 << 0;
    /// Entity bits: origin y.
    pub const U_ORIGIN2: u32 = 1 << 1;
    /// Entity bits: yaw.
    pub const U_ANGLE2: u32 = 1 << 2;
    /// Entity bits: roll.
    pub const U_ANGLE3: u32 = 1 << 3;
    /// Entity bits: 8-bit frame.
    pub const U_FRAME8: u32 = 1 << 4;
    /// Entity bits: event.
    pub const U_EVENT: u32 = 1 << 5;
    /// Entity bits: remove.
    pub const U_REMOVE: u32 = 1 << 6;
    /// Entity bits: more bits follow.
    pub const U_MOREBITS1: u32 = 1 << 7;
    /// Entity bits: 16-bit number.
    pub const U_NUMBER16: u32 = 1 << 8;
    /// Entity bits: origin z.
    pub const U_ORIGIN3: u32 = 1 << 9;
    /// Entity bits: pitch.
    pub const U_ANGLE1: u32 = 1 << 10;
    /// Entity bits: model.
    pub const U_MODEL: u32 = 1 << 11;
    /// Entity bits: 8-bit render effects.
    pub const U_RENDERFX8: u32 = 1 << 12;
    /// Entity bits: 8-bit effects.
    pub const U_EFFECTS8: u32 = 1 << 14;
    /// Entity bits: more bits follow.
    pub const U_MOREBITS2: u32 = 1 << 15;
    /// Entity bits: 8-bit skin.
    pub const U_SKIN8: u32 = 1 << 16;
    /// Entity bits: 16-bit frame.
    pub const U_FRAME16: u32 = 1 << 17;
    /// Entity bits: 16-bit render effects.
    pub const U_RENDERFX16: u32 = 1 << 18;
    /// Entity bits: 16-bit effects.
    pub const U_EFFECTS16: u32 = 1 << 19;
    /// Entity bits: second model.
    pub const U_MODEL2: u32 = 1 << 20;
    /// Entity bits: third model.
    pub const U_MODEL3: u32 = 1 << 21;
    /// Entity bits: fourth model.
    pub const U_MODEL4: u32 = 1 << 22;
    /// Entity bits: more bits follow.
    pub const U_MOREBITS3: u32 = 1 << 23;
    /// Entity bits: old origin.
    pub const U_OLDORIGIN: u32 = 1 << 24;
    /// Entity bits: 16-bit skin.
    pub const U_SKIN16: u32 = 1 << 25;
    /// Entity bits: sound.
    pub const U_SOUND: u32 = 1 << 26;
    /// Entity bits: solid.
    pub const U_SOLID: u32 = 1 << 27;

    /// Player-state bits: move type.
    pub const PS_M_TYPE: u32 = 1 << 0;
    /// Player-state bits: origin.
    pub const PS_M_ORIGIN: u32 = 1 << 1;
    /// Player-state bits: velocity.
    pub const PS_M_VELOCITY: u32 = 1 << 2;
    /// Player-state bits: time.
    pub const PS_M_TIME: u32 = 1 << 3;
    /// Player-state bits: flags.
    pub const PS_M_FLAGS: u32 = 1 << 4;
    /// Player-state bits: gravity.
    pub const PS_M_GRAVITY: u32 = 1 << 5;
    /// Player-state bits: delta angles.
    pub const PS_M_DELTA_ANGLES: u32 = 1 << 6;
    /// Player-state bits: view offset.
    pub const PS_VIEWOFFSET: u32 = 1 << 7;
    /// Player-state bits: view angles.
    pub const PS_VIEWANGLES: u32 = 1 << 8;
    /// Player-state bits: kick angles.
    pub const PS_KICKANGLES: u32 = 1 << 9;
    /// Player-state bits: blend.
    pub const PS_BLEND: u32 = 1 << 10;
    /// Player-state bits: field of view.
    pub const PS_FOV: u32 = 1 << 11;
    /// Player-state bits: weapon index.
    pub const PS_WEAPONINDEX: u32 = 1 << 12;
    /// Player-state bits: weapon frame.
    pub const PS_WEAPONFRAME: u32 = 1 << 13;
    /// Player-state bits: refresh flags.
    pub const PS_RDFLAGS: u32 = 1 << 14;
    /// Player-state bits: rerelease view height.
    pub const PS_RR_VIEWHEIGHT: u32 = 1 << 15;
}

/// Quake III wire constants.
pub mod q3 {
    /// Quake III wire version.
    pub const PROTOCOL_VERSION: u32 = 68;
    /// Maximum message length.
    pub const MAX_MESSAGE_LENGTH: usize = 16384;
    /// Maximum string characters.
    pub const MAX_STRING_CHARS: usize = 1024;
    /// Maximum big-info-string characters.
    pub const BIG_INFO_STRING: usize = 8192;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_versions_match_donor() {
        assert_eq!(ProtocolIdentity::Q1Netquake.version(), 15);
        assert_eq!(ProtocolIdentity::Q1Fitzquake.version(), 666);
        assert_eq!(ProtocolIdentity::Q1Rmq { flags: 130 }.version(), 999);
        assert_eq!(ProtocolIdentity::Q1Quakeworld.version(), 28);
        assert_eq!(ProtocolIdentity::Q1QuakeworldWide { flags: 0 }.version(), 29);
        assert_eq!(ProtocolIdentity::Q2Classic.version(), 34);
        assert_eq!(ProtocolIdentity::Q2R1q2 { revision: 1904 }.version(), 35);
        assert_eq!(ProtocolIdentity::Q2Q2pro { revision: 1026 }.version(), 36);
        assert_eq!(ProtocolIdentity::Q2Rerelease.version(), 1038);
        assert_eq!(ProtocolIdentity::Q2Kex.version(), 2023);
        assert_eq!(ProtocolIdentity::Q2KexDemo.version(), 2022);
        assert_eq!(ProtocolIdentity::Q2PrivateClassic.version(), 4038);
        assert_eq!(ProtocolIdentity::Q3.version(), 68);
    }

    #[test]
    fn opcode_and_bit_values_match_donor() {
        assert_eq!(q1::Svc::Serverinfo as u8, 11);
        assert_eq!(q1::Clc::Move as u8, 3);
        assert_eq!(q1::U_LONGENTITY, 1 << 14);
        assert_eq!(q1::SU_EXTEND3, 1 << 31);
        assert_eq!(qw::Svc::Updatepl as u8, 53);
        assert_eq!(qw::Clc::Upload as u8, 7);
        assert_eq!(qw::U_REMOVE, 1 << 14);
        assert_eq!(q2::Svc::Frame as u8, 20);
        assert_eq!(q2::Clc::Q2proUserinfoDelta as u8, 12);
        assert_eq!(q2::U_SOLID, 1 << 27);
        assert_eq!(q2::PS_RR_VIEWHEIGHT, 1 << 15);
        assert_eq!(q2::MAX_MSGLEN, 1400);
        assert_eq!(q3::MAX_MESSAGE_LENGTH, 16384);
    }
}
