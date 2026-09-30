//! Quake III foundation: events.
//!
//! Donor provenance: `src/content/q3/foundation/events.ts`.

use crate::q3anim::PlayerFootsteps;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::character::*;
use crate::q3::foundation::mirrors::*;
use crate::q3::foundation::player_pose::*;

// ---------------------------------------------------------------------------
// events.ts: CG_EntityEvent and CG_PainEvent character behavior.
// ---------------------------------------------------------------------------

/// Sound channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3SoundChannel {
    /// Auto.
    Auto,
    /// Voice.
    Voice,
    /// Body.
    Body,
}

/// Footstep material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3StepMaterial {
    /// Normal.
    Normal,
    /// Boot.
    Boot,
    /// Flesh.
    Flesh,
    /// Mech.
    Mech,
    /// Energy.
    Energy,
    /// Metal.
    Metal,
    /// Splash.
    Splash,
}

impl From<PlayerFootsteps> for Q3StepMaterial {
    fn from(footsteps: PlayerFootsteps) -> Self {
        match footsteps {
            PlayerFootsteps::Normal => Q3StepMaterial::Normal,
            PlayerFootsteps::Boot => Q3StepMaterial::Boot,
            PlayerFootsteps::Flesh => Q3StepMaterial::Flesh,
            PlayerFootsteps::Mech => Q3StepMaterial::Mech,
            PlayerFootsteps::Energy => Q3StepMaterial::Energy,
        }
    }
}

/// Teleport direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3TeleportDirection {
    /// In.
    In,
    /// Out.
    Out,
}

/// Character presentation effect (`Q3CharacterPresentationEffect`).
#[derive(Debug, Clone, PartialEq)]
pub enum Q3CharacterPresentationEffect {
    /// Custom character sound.
    CustomSound {
        /// Channel.
        channel: Q3SoundChannel,
        /// Name.
        name: String,
    },
    /// Sound path.
    Sound {
        /// Channel.
        channel: Q3SoundChannel,
        /// Path.
        path: String,
    },
    /// Footstep.
    Footstep {
        /// Material.
        material: Q3StepMaterial,
        /// Variant.
        variant: i32,
    },
    /// Jump pad smoke (radius 32, 1000ms).
    JumpPadSmoke,
    /// Teleport effect.
    Teleport {
        /// Direction.
        direction: Q3TeleportDirection,
    },
    /// Weapon fire.
    WeaponFire,
    /// Out of ammo.
    OutOfAmmo,
    /// Gib player.
    GibPlayer,
    /// Stop looping sound.
    StopLoopingSound,
    /// Retained source event for its owner.
    SourceEvent(Q3CharacterEvent),
}

/// Event presentation options (`Q3CharacterEventOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3CharacterEventOptions {
    /// Local seat.
    pub local: bool,
    /// Footsteps enabled.
    pub footsteps: bool,
    /// Predict steps.
    pub predict_steps: bool,
    /// Source flags.
    pub source_flags: i32,
}

/// Cgame random stream (`{ rand(): number }`).
pub trait Q3EventRandom {
    /// Next random value.
    fn rand(&mut self) -> i32;
}

/// Character event presenter (`Q3CharacterEventPresenter`).
#[derive(Debug)]
pub struct Q3CharacterEventPresenter<'a, R> {
    /// Step time.
    pub step_time: i32,
    /// Step change.
    pub step_change: f32,
    /// Land time.
    pub land_time: i32,
    /// Land change.
    pub land_change: f32,
    /// Muzzle flash time.
    pub muzzle_flash_time: i32,
    pose: &'a mut PlayerPoseState,
    footsteps: PlayerFootsteps,
    random: R,
}

impl<'a, R: Q3EventRandom> Q3CharacterEventPresenter<'a, R> {
    /// Bind a pose, footsteps, and random stream.
    pub fn new(pose: &'a mut PlayerPoseState, footsteps: PlayerFootsteps, random: R) -> Self {
        Self {
            step_time: 0,
            step_change: 0.0,
            land_time: 0,
            land_change: 0.0,
            muzzle_flash_time: -99999,
            pose,
            footsteps,
            random,
        }
    }

    /// Pain sound gating (`pain`).
    pub fn pain(&mut self, time_ms: i32, health: i32) -> Vec<Q3CharacterPresentationEffect> {
        if time_ms.wrapping_sub(self.pose.pain_time) < 500 {
            return Vec::new();
        }
        let level = if health < 25 {
            25
        } else if health < 50 {
            50
        } else if health < 75 {
            75
        } else {
            100
        };
        self.pose.pain_time = time_ms;
        self.pose.pain_direction = !self.pose.pain_direction;
        vec![Q3CharacterPresentationEffect::CustomSound {
            channel: Q3SoundChannel::Voice,
            name: format!("*pain{level}_1.wav"),
        }]
    }

    /// Present one source event (`event`).
    pub fn event(
        &mut self,
        source: &Q3CharacterEvent,
        options: &Q3CharacterEventOptions,
    ) -> Vec<Q3CharacterPresentationEffect> {
        use Q3CharacterPresentationEffect as Effect;
        let event = source.event & !0x300;
        let time = source.time_ms;
        match event {
            x if x == Q3EntityEvent::NONE => Vec::new(),
            x if x == Q3EntityEvent::FOOTSTEP
                || x == Q3EntityEvent::FOOTSTEP_METAL
                || x == Q3EntityEvent::FOOTSPLASH
                || x == Q3EntityEvent::FOOTWADE
                || x == Q3EntityEvent::SWIM =>
            {
                if !options.footsteps {
                    return Vec::new();
                }
                let material = if event == Q3EntityEvent::FOOTSTEP {
                    self.footsteps.into()
                } else if event == Q3EntityEvent::FOOTSTEP_METAL {
                    Q3StepMaterial::Metal
                } else {
                    Q3StepMaterial::Splash
                };
                vec![Effect::Footstep {
                    material,
                    variant: self.random.rand() & 3,
                }]
            }
            x if x == Q3EntityEvent::FALL_SHORT || x == Q3EntityEvent::FALL_MEDIUM || x == Q3EntityEvent::FALL_FAR => {
                if event == Q3EntityEvent::FALL_FAR {
                    self.pose.pain_time = time;
                }
                if options.local {
                    self.land_change = -8.0 * (event - Q3EntityEvent::FALL_SHORT + 1) as f32;
                    self.land_time = time;
                }
                if event == Q3EntityEvent::FALL_SHORT {
                    vec![Effect::Sound {
                        channel: Q3SoundChannel::Auto,
                        path: "sound/player/land1.wav".to_string(),
                    }]
                } else if event == Q3EntityEvent::FALL_MEDIUM {
                    vec![Effect::CustomSound {
                        channel: Q3SoundChannel::Voice,
                        name: "*pain100_1.wav".to_string(),
                    }]
                } else {
                    vec![Effect::CustomSound {
                        channel: Q3SoundChannel::Auto,
                        name: "*fall1.wav".to_string(),
                    }]
                }
            }
            x if x == Q3EntityEvent::STEP_4
                || x == Q3EntityEvent::STEP_8
                || x == Q3EntityEvent::STEP_12
                || x == Q3EntityEvent::STEP_16 =>
            {
                if !options.local || !options.predict_steps {
                    return Vec::new();
                }
                let elapsed = time.wrapping_sub(self.step_time);
                let previous = if elapsed < 200 {
                    self.step_change * 200_i32.wrapping_sub(elapsed) as f32 / 200.0
                } else {
                    0.0
                };
                self.step_change = (previous + 4.0 * (event - Q3EntityEvent::STEP_4 + 1) as f32).min(32.0);
                self.step_time = time;
                Vec::new()
            }
            x if x == Q3EntityEvent::JUMP_PAD => vec![
                Effect::JumpPadSmoke,
                Effect::Sound {
                    channel: Q3SoundChannel::Voice,
                    path: "sound/world/jumppad.wav".to_string(),
                },
                Effect::CustomSound {
                    channel: Q3SoundChannel::Voice,
                    name: "*jump1.wav".to_string(),
                },
            ],
            x if x == Q3EntityEvent::JUMP => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Voice,
                name: "*jump1.wav".to_string(),
            }],
            x if x == Q3EntityEvent::TAUNT => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Voice,
                name: "*taunt.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_TOUCH => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_in.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_LEAVE => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_out.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_UNDER => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_un.wav".to_string(),
            }],
            x if x == Q3EntityEvent::WATER_CLEAR => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Auto,
                name: "*gasp.wav".to_string(),
            }],
            x if x == Q3EntityEvent::NOAMMO => {
                if options.local {
                    vec![Effect::OutOfAmmo]
                } else {
                    Vec::new()
                }
            }
            x if x == Q3EntityEvent::CHANGE_WEAPON => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/weapons/change.wav".to_string(),
            }],
            x if x == Q3EntityEvent::FIRE_WEAPON => {
                self.muzzle_flash_time = time;
                vec![Effect::WeaponFire]
            }
            x if x == Q3EntityEvent::PLAYER_TELEPORT_IN => vec![
                Effect::Sound {
                    channel: Q3SoundChannel::Auto,
                    path: "sound/world/telein.wav".to_string(),
                },
                Effect::Teleport {
                    direction: Q3TeleportDirection::In,
                },
            ],
            x if x == Q3EntityEvent::PLAYER_TELEPORT_OUT => vec![
                Effect::Sound {
                    channel: Q3SoundChannel::Auto,
                    path: "sound/world/teleout.wav".to_string(),
                },
                Effect::Teleport {
                    direction: Q3TeleportDirection::Out,
                },
            ],
            x if x == Q3EntityEvent::PAIN => {
                if options.local {
                    Vec::new()
                } else {
                    self.pain(time, source.parameter)
                }
            }
            x if x == Q3EntityEvent::DEATH1 || x == Q3EntityEvent::DEATH2 || x == Q3EntityEvent::DEATH3 => {
                vec![Effect::CustomSound {
                    channel: Q3SoundChannel::Voice,
                    name: format!("*death{}.wav", event - Q3EntityEvent::DEATH1 + 1),
                }]
            }
            x if x == Q3EntityEvent::GIB_PLAYER => {
                if options.source_flags & 0x200 != 0 {
                    vec![Effect::GibPlayer]
                } else {
                    vec![
                        Effect::Sound {
                            channel: Q3SoundChannel::Body,
                            path: "sound/player/gibsplt1.wav".to_string(),
                        },
                        Effect::GibPlayer,
                    ]
                }
            }
            x if x == Q3EntityEvent::STOPLOOPINGSOUND => vec![Effect::StopLoopingSound],
            _ => vec![Effect::SourceEvent(source.clone())],
        }
    }
}
