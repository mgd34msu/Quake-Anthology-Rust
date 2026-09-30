//! Quake III foundation: events.
//!
//! Donor provenance: `src/content/q3/foundation/events.ts`.

use crate::q3anim::PlayerFootsteps;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::character::*;
use crate::q3::foundation::player_pose::*;
use qa_world::movement::q3::constants::entity_event;

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
            x if x == entity_event::NONE => Vec::new(),
            x if x == entity_event::FOOTSTEP
                || x == entity_event::FOOTSTEP_METAL
                || x == entity_event::FOOTSPLASH
                || x == entity_event::FOOTWADE
                || x == entity_event::SWIM =>
            {
                if !options.footsteps {
                    return Vec::new();
                }
                let material = if event == entity_event::FOOTSTEP {
                    self.footsteps.into()
                } else if event == entity_event::FOOTSTEP_METAL {
                    Q3StepMaterial::Metal
                } else {
                    Q3StepMaterial::Splash
                };
                vec![Effect::Footstep {
                    material,
                    variant: self.random.rand() & 3,
                }]
            }
            x if x == entity_event::FALL_SHORT || x == entity_event::FALL_MEDIUM || x == entity_event::FALL_FAR => {
                if event == entity_event::FALL_FAR {
                    self.pose.pain_time = time;
                }
                if options.local {
                    self.land_change = -8.0 * (event - entity_event::FALL_SHORT + 1) as f32;
                    self.land_time = time;
                }
                if event == entity_event::FALL_SHORT {
                    vec![Effect::Sound {
                        channel: Q3SoundChannel::Auto,
                        path: "sound/player/land1.wav".to_string(),
                    }]
                } else if event == entity_event::FALL_MEDIUM {
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
            x if x == entity_event::STEP_4
                || x == entity_event::STEP_8
                || x == entity_event::STEP_12
                || x == entity_event::STEP_16 =>
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
                self.step_change = (previous + 4.0 * (event - entity_event::STEP_4 + 1) as f32).min(32.0);
                self.step_time = time;
                Vec::new()
            }
            x if x == entity_event::JUMP_PAD => vec![
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
            x if x == entity_event::JUMP => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Voice,
                name: "*jump1.wav".to_string(),
            }],
            x if x == entity_event::TAUNT => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Voice,
                name: "*taunt.wav".to_string(),
            }],
            x if x == entity_event::WATER_TOUCH => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_in.wav".to_string(),
            }],
            x if x == entity_event::WATER_LEAVE => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_out.wav".to_string(),
            }],
            x if x == entity_event::WATER_UNDER => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/player/watr_un.wav".to_string(),
            }],
            x if x == entity_event::WATER_CLEAR => vec![Effect::CustomSound {
                channel: Q3SoundChannel::Auto,
                name: "*gasp.wav".to_string(),
            }],
            x if x == entity_event::NOAMMO => {
                if options.local {
                    vec![Effect::OutOfAmmo]
                } else {
                    Vec::new()
                }
            }
            x if x == entity_event::CHANGE_WEAPON => vec![Effect::Sound {
                channel: Q3SoundChannel::Auto,
                path: "sound/weapons/change.wav".to_string(),
            }],
            x if x == entity_event::FIRE_WEAPON => {
                self.muzzle_flash_time = time;
                vec![Effect::WeaponFire]
            }
            x if x == entity_event::PLAYER_TELEPORT_IN => vec![
                Effect::Sound {
                    channel: Q3SoundChannel::Auto,
                    path: "sound/world/telein.wav".to_string(),
                },
                Effect::Teleport {
                    direction: Q3TeleportDirection::In,
                },
            ],
            x if x == entity_event::PLAYER_TELEPORT_OUT => vec![
                Effect::Sound {
                    channel: Q3SoundChannel::Auto,
                    path: "sound/world/teleout.wav".to_string(),
                },
                Effect::Teleport {
                    direction: Q3TeleportDirection::Out,
                },
            ],
            x if x == entity_event::PAIN => {
                if options.local {
                    Vec::new()
                } else {
                    self.pain(time, source.parameter)
                }
            }
            x if x == entity_event::DEATH1 || x == entity_event::DEATH2 || x == entity_event::DEATH3 => {
                vec![Effect::CustomSound {
                    channel: Q3SoundChannel::Voice,
                    name: format!("*death{}.wav", event - entity_event::DEATH1 + 1),
                }]
            }
            x if x == entity_event::GIB_PLAYER => {
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
            x if x == entity_event::STOPLOOPINGSOUND => vec![Effect::StopLoopingSound],
            _ => vec![Effect::SourceEvent(source.clone())],
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::q3anim::PlayerFootsteps;
    use qa_core::identity::{IdentityOwner, OwnedActor, ProviderId};

    use super::*;
    fn test_actor() -> (OwnedActor, ProviderId) {
        let owner = IdentityOwner::create("test").unwrap();
        let provider = ProviderId::new("q3", "test");
        let owned = owner.owned_actor(&owner.actor(3, 1), provider.clone()).unwrap();
        (owned, provider)
    }
    struct StepRandom {
        value: i32,
    }

    impl Q3EventRandom for StepRandom {
        fn rand(&mut self) -> i32 {
            self.value
        }
    }
    fn event_fixture(event: i32, parameter: i32) -> Q3CharacterEvent {
        let (actor, _) = test_actor();
        Q3CharacterEvent {
            actor,
            sequence: 4,
            time_ms: 2000,
            event,
            parameter,
        }
    }
    #[test]
    fn event_presenter_covers_character_events() {
        let mut pose = create_player_pose_state();
        let options = Q3CharacterEventOptions {
            local: false,
            footsteps: true,
            predict_steps: true,
            source_flags: 0,
        };
        let mut presenter = Q3CharacterEventPresenter::new(&mut pose, PlayerFootsteps::Boot, StepRandom { value: 7 });
        assert!(presenter.pain(100, 90).is_empty());
        let pain = presenter.pain(600, 20);
        assert_eq!(pain.len(), 1);
        assert!(matches!(
            &pain[0],
            Q3CharacterPresentationEffect::CustomSound { name, .. } if name == "*pain25_1.wav"
        ));

        let steps = presenter.event(&event_fixture(entity_event::FOOTSTEP, 0), &options);
        assert!(matches!(
            &steps[0],
            Q3CharacterPresentationEffect::Footstep {
                material: Q3StepMaterial::Boot,
                variant: 3
            }
        ));
        let metal = presenter.event(&event_fixture(entity_event::FOOTSTEP_METAL, 0), &options);
        assert!(matches!(
            &metal[0],
            Q3CharacterPresentationEffect::Footstep {
                material: Q3StepMaterial::Metal,
                ..
            }
        ));
        let quiet = Q3CharacterEventOptions {
            footsteps: false,
            ..options
        };
        assert!(presenter
            .event(&event_fixture(entity_event::FOOTSTEP, 0), &quiet)
            .is_empty());

        let fire = presenter.event(&event_fixture(entity_event::FIRE_WEAPON, 0), &options);
        assert_eq!(fire, vec![Q3CharacterPresentationEffect::WeaponFire]);
        assert_eq!(presenter.muzzle_flash_time, 2000);

        let death = presenter.event(&event_fixture(entity_event::DEATH2, 0), &options);
        assert!(matches!(
            &death[0],
            Q3CharacterPresentationEffect::CustomSound { name, .. } if name == "*death2.wav"
        ));

        let gib = presenter.event(&event_fixture(entity_event::GIB_PLAYER, 0), &options);
        assert_eq!(gib.len(), 2);
        let flagged = Q3CharacterEventOptions {
            source_flags: 0x200,
            ..options
        };
        let gib = presenter.event(&event_fixture(entity_event::GIB_PLAYER, 0), &flagged);
        assert_eq!(gib, vec![Q3CharacterPresentationEffect::GibPlayer]);

        let remote_pain = presenter.event(&event_fixture(entity_event::PAIN, 60), &options);
        assert!(!remote_pain.is_empty());
        let local = Q3CharacterEventOptions { local: true, ..options };
        assert!(presenter
            .event(&event_fixture(entity_event::PAIN, 60), &local)
            .is_empty());
        let tele = presenter.event(&event_fixture(entity_event::PLAYER_TELEPORT_OUT, 0), &options);
        assert_eq!(tele.len(), 2);
        let kept = presenter.event(&event_fixture(entity_event::OBITUARY, 9), &options);
        assert!(matches!(
            &kept[0],
            Q3CharacterPresentationEffect::SourceEvent(event) if event.parameter == 9
        ));
        let nop = presenter.event(&event_fixture(0x300 | entity_event::JUMP, 0), &options);
        assert_eq!(nop.len(), 1);

        let mut pose = create_player_pose_state();
        let mut presenter = Q3CharacterEventPresenter::new(&mut pose, PlayerFootsteps::Normal, StepRandom { value: 0 });
        let local = Q3CharacterEventOptions { local: true, ..options };
        presenter.event(&event_fixture(entity_event::FALL_FAR, 0), &local);
        assert_eq!(presenter.land_time, 2000);
        assert_eq!(presenter.land_change, -24.0);
        presenter.event(&event_fixture(entity_event::STEP_8, 0), &local);
        assert_eq!(presenter.step_time, 2000);
        assert!(presenter.step_change > 0.0);
        let pad = presenter.event(&event_fixture(entity_event::JUMP_PAD, 0), &options);
        assert_eq!(pad.len(), 3);
        assert!(matches!(pad[0], Q3CharacterPresentationEffect::JumpPadSmoke));
    }
}
