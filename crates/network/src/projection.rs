//! One field table converts between common players and native record layouts.
//! Native ordinals and effective tuning are supplied by the module/connection.
use crate::{commands::packet::Protocol, states};
use qa_core::{
    math::{AngleShortForm, angle_to_short, short_to_angle},
    primitives::{
        EntityId, MovementMode, MovementTimer, PlayerState, PlayerTail, ValueBinding, ValueWidth,
    },
};

/// These are native values, never common entity, client or registry handles.
pub struct PlayerContext {
    pub client_number: Option<u32>,
    pub ground_number: Option<u32>,
    pub weapon_number: Option<u32>,
    pub weapon_model: Option<u32>,
    pub gravity: f32,
    pub speed: f32,
    /// Visibility/self/spectator selection belongs to the QW snapshot provider.
    pub player_info_flags: u16,
    pub command_age_ms: u32,
    /// Native entity yaw may differ from the player's command/view angles.
    pub body_yaw: f32,
}

#[derive(Clone, Copy)]
enum Source {
    Position(usize),
    Velocity(usize),
    View(usize),
    ViewOffset(usize),
    Punch(usize),
    DeltaAngle(usize),
    CommandTime,
    Timer,
    Flags,
    Mode,
    Health,
    Armor,
    Score,
    Frags,
    Grounded,
    InWater,
    WeaponTime,
    WeaponFrame,
    Client,
    Ground,
    Weapon,
    WeaponModel,
    Gravity,
    Speed,
    InfoFlags,
    CommandAge,
    IdealPitch,
    BodyYaw,
}
#[derive(Clone, Copy)]
enum Convert {
    Bits,
    IntegerFloat,
    FloatInteger,
    Eighth,
    ShortAngle,
    UnsignedShortAngle,
    TimerEighth,
}
#[derive(Clone, Copy)]
struct Field {
    word: usize,
    source: Source,
    convert: Convert,
}
const fn f(word: usize, source: Source, convert: Convert) -> Field {
    Field {
        word,
        source,
        convert,
    }
}
use Convert::*;
use Source::*;
static NQ: &[Field] = &[
    f(0, ViewOffset(2), Bits),
    f(1, IdealPitch, Bits),
    f(2, Punch(0), Bits),
    f(3, Velocity(0), Bits),
    f(4, Punch(1), Bits),
    f(5, Velocity(1), Bits),
    f(6, Punch(2), Bits),
    f(7, Velocity(2), Bits),
    f(10, Armor, IntegerFloat),
    f(11, WeaponModel, Bits),
    f(12, Health, IntegerFloat),
    f(18, Weapon, Bits),
    f(19, Grounded, Bits),
    f(20, InWater, Bits),
];
static QW: &[Field] = &[
    f(0, Position(0), Bits),
    f(1, Position(1), Bits),
    f(2, Position(2), Bits),
    f(4, CommandAge, Bits),
    f(5, Velocity(0), Bits),
    f(6, Velocity(1), Bits),
    f(7, Velocity(2), Bits),
    f(12, InfoFlags, Bits),
    f(13, BodyYaw, Bits),
];
static Q2: &[Field] = &[
    f(0, Mode, Bits),
    f(1, Position(0), Eighth),
    f(2, Position(1), Eighth),
    f(3, Position(2), Eighth),
    f(4, Velocity(0), Eighth),
    f(5, Velocity(1), Eighth),
    f(6, Velocity(2), Eighth),
    f(7, Timer, TimerEighth),
    f(8, Flags, Bits),
    f(9, Gravity, FloatInteger),
    f(10, DeltaAngle(0), ShortAngle),
    f(11, DeltaAngle(1), ShortAngle),
    f(12, DeltaAngle(2), ShortAngle),
    f(13, ViewOffset(0), Bits),
    f(14, ViewOffset(1), Bits),
    f(15, ViewOffset(2), Bits),
    f(16, View(0), Bits),
    f(17, View(1), Bits),
    f(18, View(2), Bits),
    f(19, Punch(0), Bits),
    f(20, Punch(1), Bits),
    f(21, Punch(2), Bits),
    f(22, WeaponModel, Bits),
    f(23, WeaponFrame, Bits),
    f(37, Health, Bits),
    f(41, Armor, Bits),
    f(50, Frags, Bits),
];
static Q3: &[Field] = &[
    f(0, CommandTime, Bits),
    f(1, Position(0), Bits),
    f(2, Position(1), Bits),
    f(4, Velocity(0), Bits),
    f(5, Velocity(1), Bits),
    f(6, View(1), Bits),
    f(7, View(0), Bits),
    f(8, WeaponTime, Bits),
    f(9, Position(2), Bits),
    f(10, Velocity(2), Bits),
    f(12, Timer, Bits),
    f(19, Flags, Bits),
    f(20, Ground, Bits),
    f(24, Gravity, FloatInteger),
    f(25, Speed, FloatInteger),
    f(26, DeltaAngle(1), UnsignedShortAngle),
    f(28, ViewOffset(2), FloatInteger),
    f(34, Mode, Bits),
    f(35, DeltaAngle(0), UnsignedShortAngle),
    f(36, DeltaAngle(2), UnsignedShortAngle),
    f(40, Client, Bits),
    f(41, Weapon, Bits),
    f(42, View(2), Bits),
    f(48, Health, Bits),
    f(51, Armor, Bits),
    f(64, Score, Bits),
];

/// One walker; the native format selects data at load, not another player store.
pub struct PlayerProjection {
    fields: &'static [Field],
    words: usize,
    modes: [u32; 7],
    received_modes: &'static [MovementMode],
    flags: [u32; 7],
    bindings: Box<[(usize, ValueBinding)]>,
    pub dropped_bindings: usize,
}
impl PlayerProjection {
    pub fn load(protocol: Protocol, bindings: &[(usize, ValueBinding)]) -> Self {
        use MovementMode as M;
        let (fields, words, modes, received_modes, flags) = match protocol {
            Protocol::NetQuake15 => (NQ, states::NQ_PLAYER_WORDS, [0; 7], &[][..], [0; 7]),
            Protocol::QuakeWorld28 => (QW, states::QW_PLAYER_WORDS, [0; 7], &[][..], [0; 7]),
            Protocol::Quake2_34 => (
                Q2,
                states::Q2_PLAYER_WORDS,
                [0, 1, 1, 1, 2, 3, 4],
                &[M::Walk, M::Spectator, M::Dead, M::Gib, M::Frozen][..],
                [1, 2, 4, 8, 16, 32, 0],
            ),
            Protocol::Quake3_68 => (
                Q3,
                states::PLAYER_WORDS,
                [0, 1, 1, 2, 3, 3, 4],
                &[
                    M::Walk,
                    M::Noclip,
                    M::Spectator,
                    M::Dead,
                    M::Frozen,
                    M::Frozen,
                    M::Frozen,
                ][..],
                [1, 2, 0, 256, 32, 0, 64],
            ),
        };
        let admitted: Box<[_]> = bindings
            .iter()
            .copied()
            .filter(|(word, _)| *word < words)
            .collect();
        Self {
            fields,
            words,
            modes,
            received_modes,
            flags,
            dropped_bindings: bindings.len() - admitted.len(),
            bindings: admitted,
        }
    }
    pub fn words(&self) -> usize {
        self.words
    }

    /// The module can supply fields absent from the common hot columns through
    /// load-resolved numeric bindings. Last binding wins in registration order.
    pub fn reduce(&self, player: &PlayerState, context: &PlayerContext, out: &mut [u32]) -> bool {
        let Some(out) = out.get_mut(..self.words) else {
            return false;
        };
        out.fill(0);
        for field in self.fields {
            let word = self.value(field.source, player, context);
            out[field.word] = match field.convert {
                Bits => word,
                IntegerFloat => (word as i32 as f32).to_bits(),
                FloatInteger => f32::from_bits(word) as i32 as u32,
                Eighth => (f32::from_bits(word) * 8.0) as i32 as i16 as i32 as u32,
                ShortAngle => angle_to_short(f32::from_bits(word), AngleShortForm::MultiplyDivide)
                    as i16 as i32 as u32,
                UnsignedShortAngle => {
                    angle_to_short(f32::from_bits(word), AngleShortForm::MultiplyDivide) as u16
                        as u32
                }
                TimerEighth => (word / 8).min(255),
            };
        }
        for &(word, binding) in &self.bindings {
            if let Some(value) = binding.export(&player.values) {
                out[word] = if binding.width == ValueWidth::Signed16 {
                    value as i16 as i32 as u32
                } else {
                    value
                };
            }
        }
        true
    }

    /// Apply words returned by the existing native decoder, not the writer's
    /// input words (NQ's QC float stats decode to native signed integers).
    /// Roles, untransmitted fields and owned arenas remain unchanged. Ordinals
    /// stay in boundary context; the caller resolves ground in its namespace.
    /// Effective gravity/speed stay in context for the movement adapter, which
    /// must account for its independently selected tuning/multipliers.
    pub fn apply(
        &self,
        words: &[u32],
        player: &mut PlayerState,
        context: &mut PlayerContext,
        mut resolve_ground: impl FnMut(u32) -> Option<EntityId>,
    ) -> bool {
        let Some(words) = words.get(..self.words) else {
            return false;
        };
        for field in self.fields {
            let word = words[field.word];
            let value = match field.convert {
                Bits | IntegerFloat => word,
                FloatInteger => (word as i32 as f32).to_bits(),
                Eighth => (f32::from(word as i16) * 0.125).to_bits(),
                ShortAngle => short_to_angle(word as i16 as i32).to_bits(),
                UnsignedShortAngle => short_to_angle(word as u16 as i32).to_bits(),
                TimerEighth => u32::from(word as u8) * 8,
            };
            match field.source {
                Position(i) => player.body.position.0[i] = f32::from_bits(value),
                Velocity(i) => player.body.velocity.0[i] = f32::from_bits(value),
                View(i) => player.view_angles.0[i] = f32::from_bits(value),
                ViewOffset(i) => player.view_offset.0[i] = f32::from_bits(value),
                Punch(i) => player.punch_angles.0[i] = f32::from_bits(value),
                DeltaAngle(i) => player.movement.delta_angles.0[i] = f32::from_bits(value),
                CommandTime => player.movement.command_time_ms = value as i32,
                Timer => player.movement.remaining_ms = value,
                Flags => {
                    if self.flags[0] != 0 {
                        player.movement.ducked = value & self.flags[0] != 0;
                    }
                    if self.flags[1] != 0 {
                        player.movement.jump_held = value & self.flags[1] != 0;
                    }
                    if self.flags[2] != 0 {
                        player.movement.grounded = value & self.flags[2] != 0;
                        // Q2 transmits contact presence, not a ground ordinal.
                        player.movement.ground = None;
                    }
                    for (timer, flag) in [
                        MovementTimer::WATER_JUMP,
                        MovementTimer::LAND,
                        MovementTimer::TELEPORT,
                        MovementTimer::KNOCKBACK,
                    ]
                    .into_iter()
                    .zip(&self.flags[3..])
                    {
                        if *flag != 0 {
                            player.movement.timer.0 = (player.movement.timer.0 & !timer.0)
                                | if value & flag != 0 { timer.0 } else { 0 };
                        }
                    }
                }
                Mode => {
                    // Preserve a caller's Fly/Noclip/Gib choice when the wire
                    // cannot distinguish it from another common mode.
                    if self.native_mode(player.movement.mode) != value {
                        player.movement.mode = self
                            .received_modes
                            .get(value as usize)
                            .copied()
                            .unwrap_or(MovementMode::Frozen);
                    }
                }
                Health => player.health = value as i32,
                Armor => player.armor = value as i32,
                Score => player.score = value as i32,
                Frags => player.frags = value as i32,
                Grounded => {
                    player.movement.grounded = value != 0;
                    player.movement.ground = None;
                }
                InWater => {
                    // One native bit cannot supply the exact three-level probe.
                    player.movement.water_level = if value != 0 {
                        player.movement.water_level.max(2)
                    } else {
                        player.movement.water_level.min(1)
                    };
                }
                WeaponTime => {
                    if let PlayerTail::Q3 { weapon_time } = &mut player.tail {
                        *weapon_time = value as i32;
                    }
                }
                WeaponFrame => {
                    if let PlayerTail::Q2 { weapon_frame } = &mut player.tail {
                        *weapon_frame = value as i32;
                    }
                }
                Client => context.client_number = (value < 64).then_some(value),
                Ground => {
                    context.ground_number = (value < 1023).then_some(value);
                    player.movement.grounded = context.ground_number.is_some();
                    player.movement.ground = context.ground_number.and_then(&mut resolve_ground);
                }
                Weapon => context.weapon_number = Some(value),
                WeaponModel => context.weapon_model = Some(value),
                Gravity => context.gravity = f32::from_bits(value),
                Speed => context.speed = f32::from_bits(value),
                InfoFlags => context.player_info_flags = value as u16,
                CommandAge => context.command_age_ms = value,
                IdealPitch => player.ideal_pitch = f32::from_bits(value),
                // QW body yaw is an input to command sanitization, not a field
                // received in playerinfo. Its decoded placeholder is unused.
                BodyYaw => {}
            }
        }
        for &(word, binding) in &self.bindings {
            binding.import(&mut player.values, words[word]);
        }
        true
    }

    fn native_mode(&self, mode: MovementMode) -> u32 {
        self.modes[match mode {
            MovementMode::Walk => 0,
            MovementMode::Fly => 1,
            MovementMode::Noclip => 2,
            MovementMode::Spectator => 3,
            MovementMode::Dead => 4,
            MovementMode::Gib => 5,
            MovementMode::Frozen => 6,
        }]
    }

    fn value(&self, source: Source, p: &PlayerState, c: &PlayerContext) -> u32 {
        match source {
            Position(i) => p.body.position.0[i].to_bits(),
            Velocity(i) => p.body.velocity.0[i].to_bits(),
            View(i) => p.view_angles.0[i].to_bits(),
            ViewOffset(i) => p.view_offset.0[i].to_bits(),
            Punch(i) => p.punch_angles.0[i].to_bits(),
            DeltaAngle(i) => p.movement.delta_angles.0[i].to_bits(),
            CommandTime => p.movement.command_time_ms as u32,
            Timer => p.movement.remaining_ms,
            Flags => {
                let enabled = [
                    p.movement.ducked,
                    p.movement.jump_held,
                    p.movement.grounded,
                    p.movement.timer.contains(MovementTimer::WATER_JUMP),
                    p.movement.timer.contains(MovementTimer::LAND),
                    p.movement.timer.contains(MovementTimer::TELEPORT),
                    p.movement.timer.contains(MovementTimer::KNOCKBACK),
                ];
                enabled
                    .into_iter()
                    .zip(self.flags)
                    .fold(0, |bits, (on, flag)| bits | if on { flag } else { 0 })
            }
            Mode => self.native_mode(p.movement.mode),
            Health => p.health as u32,
            Armor => p.armor as u32,
            Score => p.score as u32,
            Frags => p.frags as u32,
            Grounded => u32::from(p.movement.grounded),
            InWater => u32::from(p.movement.water_level >= 2),
            WeaponTime => {
                if let PlayerTail::Q3 { weapon_time } = p.tail {
                    weapon_time as u32
                } else {
                    0
                }
            }
            WeaponFrame => {
                if let PlayerTail::Q2 { weapon_frame } = p.tail {
                    weapon_frame as u32
                } else {
                    0
                }
            }
            Client => c.client_number.filter(|&n| n < 64).unwrap_or(0),
            Ground => c.ground_number.filter(|&n| n < 1024).unwrap_or(1023),
            Weapon => c.weapon_number.unwrap_or(0),
            WeaponModel => c.weapon_model.unwrap_or(0),
            Gravity => c.gravity.to_bits(),
            Speed => c.speed.to_bits(),
            InfoFlags => u32::from(c.player_info_flags),
            CommandAge => c.command_age_ms.min(255),
            IdealPitch => p.ideal_pitch.to_bits(),
            BodyYaw => c.body_yaw.to_bits(),
        }
    }
}
