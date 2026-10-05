//! Player presentation and lifecycle (`src/content/q1/base/player.ts`).
//!
//! player.qc/client.qc presentation and lifecycle. Copyright (C) 1996-2022
//! id Software LLC. GPL-2.0-or-later.
//!
//! The donor keys characters off the game through a `WeakMap`, used
//! only to read the current water level from death-bubble timers. The
//! session-owned [`Q1CharacterActor`] therefore publishes its water
//! level into a small game-and-actor keyed table, cleaned on actor
//! release and character drop, mirroring the donor lookup chain.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::{Mutex, OnceLock};

use qa_core::identity::{ActorId, OwnedActor};
use qa_core::math::{Bounds, Vec3};

use crate::q1::base::map_entities::spawn_bubble;
use crate::q1::base::projectiles::throw_gib;
use crate::q1::foundation::checkpoint::{decode_checkpoint_value, encode_checkpoint_value};
use crate::q1::foundation::entity_services::{Q1DamageParams, Q1EntityServices};
use crate::q1::foundation::gameplay::BodyState;
use crate::q1::foundation::host::Q1ReleaseHook;
use crate::q1::foundation::types::{
    length, normalize, vadd, vscale, Q1CharacterAttack, Q1MoveType, Q1Powerup, Q1Solid, Q1SoundChannel,
};
use crate::q1::Q1Error;
use crate::value::{boolean, int, num, obj, str as save_str, SaveReader};

fn water_table() -> &'static Mutex<HashMap<(usize, ActorId), i32>> {
    static TABLE: OnceLock<Mutex<HashMap<(usize, ActorId), i32>>> = OnceLock::new();
    TABLE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock_water_table() -> std::sync::MutexGuard<'static, HashMap<(usize, ActorId), i32>> {
    water_table().lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn game_key(game: &Q1EntityServices) -> usize {
    std::ptr::from_ref(game) as usize
}

/// Character life (`Q1PlayerLife`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1PlayerLife {
    /// Alive.
    Alive,
    /// Dying.
    Dying,
    /// Dead.
    Dead,
    /// Respawnable.
    Respawnable,
}

impl Q1PlayerLife {
    /// Donor life text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1PlayerLife::Alive => "alive",
            Q1PlayerLife::Dying => "dying",
            Q1PlayerLife::Dead => "dead",
            Q1PlayerLife::Respawnable => "respawnable",
        }
    }
}

/// Character water type (`Q1CharacterInput["waterType"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q1CharacterWater {
    /// Not in liquid.
    Empty,
    /// Water.
    Water,
    /// Slime.
    Slime,
    /// Lava.
    Lava,
}

impl Q1CharacterWater {
    /// Donor water text.
    #[must_use]
    pub fn as_str(&self) -> &'static str {
        match self {
            Q1CharacterWater::Empty => "empty",
            Q1CharacterWater::Water => "water",
            Q1CharacterWater::Slime => "slime",
            Q1CharacterWater::Lava => "lava",
        }
    }
}

/// Per-frame character input (`Q1CharacterInput`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CharacterInput {
    /// Axe pose.
    pub axe_pose: bool,
    /// Attack held.
    pub attack: bool,
    /// Jump held.
    pub jump: bool,
    /// Use held.
    pub use_input: bool,
    /// Water level (`0..=3`).
    pub water_level: i32,
    /// Water type.
    pub water_type: Q1CharacterWater,
    /// Invisible.
    pub invisible: bool,
    /// Invulnerable.
    pub invulnerable: bool,
}

impl Default for Q1CharacterInput {
    fn default() -> Self {
        Self {
            axe_pose: false,
            attack: false,
            jump: false,
            use_input: false,
            water_level: 0,
            water_type: Q1CharacterWater::Empty,
            invisible: false,
            invulnerable: false,
        }
    }
}

/// Character presentation (`Q1CharacterPresentation`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CharacterPresentation {
    /// Model path.
    pub model: String,
    /// Model frame.
    pub frame: f64,
    /// View offset.
    pub view_offset: Vec3,
    /// Life.
    pub life: Q1PlayerLife,
    /// Solidity.
    pub solid: Q1Solid,
    /// Movement.
    pub movement: Q1MoveType,
    /// Whether the weapon shows.
    pub weapon_visible: bool,
}

/// Character lifecycle (`Q1CharacterActor["lifecycle"]`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CharacterLifecycle {
    /// Life.
    pub life: Q1PlayerLife,
    /// View offset.
    pub view_offset: Vec3,
}

/// Character frame range (`Q1CharacterFrameRange`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CharacterFrameRange {
    /// First frame.
    pub first: f64,
    /// Frame count.
    pub count: f64,
}

/// Alternate character model layout (`Q1CharacterDefinition`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q1CharacterDefinition {
    /// Model path.
    pub model: String,
    /// Stand frames.
    pub stand: Q1CharacterFrameRange,
    /// Run frames.
    pub run: Q1CharacterFrameRange,
    /// Pain frames.
    pub pain: Q1CharacterFrameRange,
    /// Death frames.
    pub death: Q1CharacterFrameRange,
}

/// Source pose (`Q1CharacterSourcePose`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Q1CharacterSourcePose {
    /// Axe pose override.
    pub axe_pose: Option<bool>,
    /// Alternate model layout.
    pub definition: Option<Q1CharacterDefinition>,
    /// Source-owned attack frame.
    pub frame: Option<f64>,
}

/// Character options (`Q1CharacterOptions`).
#[derive(Default)]
pub struct Q1CharacterOptions {
    /// Death-drop inventory conversion.
    pub drop_inventory: Option<fn()>,
    /// Respawn placement request.
    pub request_respawn: Option<fn()>,
    /// Source pose probe.
    pub source_pose: Option<fn() -> Q1CharacterSourcePose>,
    /// Fall-damage probe.
    pub fall_damage_allowed: Option<fn() -> bool>,
}

/// Animation end (`PlayerAnimation["end"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum AnimationEnd {
    /// Return to locomotion.
    Locomotion,
    /// Stay dead.
    Dead,
}

/// Player animation (`PlayerAnimation`).
#[derive(Debug, Clone, Copy, PartialEq)]
struct PlayerAnimation {
    /// First frame.
    first: f64,
    /// Frame count.
    count: f64,
    /// End behavior.
    end: AnimationEnd,
}

const DEATHS: &[PlayerAnimation] = &[
    PlayerAnimation {
        first: 50.0,
        count: 11.0,
        end: AnimationEnd::Dead,
    },
    PlayerAnimation {
        first: 61.0,
        count: 9.0,
        end: AnimationEnd::Dead,
    },
    PlayerAnimation {
        first: 70.0,
        count: 15.0,
        end: AnimationEnd::Dead,
    },
    PlayerAnimation {
        first: 85.0,
        count: 9.0,
        end: AnimationEnd::Dead,
    },
    PlayerAnimation {
        first: 94.0,
        count: 9.0,
        end: AnimationEnd::Dead,
    },
    PlayerAnimation {
        first: 94.0,
        count: 9.0,
        end: AnimationEnd::Dead,
    },
];

/// Character locomotion (`Q1CharacterActor["locomotion"]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Locomotion {
    /// Stand.
    Stand,
    /// Run.
    Run,
}

/// A character on an existing actor. It does not allocate actors,
/// bind combat, or install an arsenal (`Q1CharacterActor`).
pub struct Q1CharacterActor {
    /// Character actor.
    pub actor: OwnedActor,
    /// Character options.
    pub options: Q1CharacterOptions,
    game_key: Option<usize>,
    input: Q1CharacterInput,
    life: Q1PlayerLife,
    model: String,
    model_frame: f64,
    animation: Option<PlayerAnimation>,
    animation_frame: f64,
    attack_animation: bool,
    walk_frame: f64,
    locomotion: Option<Locomotion>,
    next_animation: f64,
    pain_until: f64,
    view_offset: Vec3,
    last_fall_speed: f64,
    air_finished: f64,
    drown_damage: f64,
    hazard_at: f64,
    in_water: bool,
}

impl Drop for Q1CharacterActor {
    fn drop(&mut self) {
        if let Some(key) = self.game_key {
            lock_water_table().remove(&(key, self.actor.id().clone()));
        }
    }
}

impl Q1CharacterActor {
    /// Bind a character to an admitted actor.
    pub fn new(game: &Q1EntityServices, actor: OwnedActor, options: Q1CharacterOptions) -> Result<Self, Q1Error> {
        game.host.actors.assert_owned(&actor)?;
        let key = game_key(game);
        lock_water_table().insert((key, actor.id().clone()), 0);
        Ok(Self {
            actor,
            options,
            game_key: Some(key),
            input: Q1CharacterInput::default(),
            life: Q1PlayerLife::Alive,
            model: String::from("progs/player.mdl"),
            model_frame: 12.0,
            animation: None,
            animation_frame: 0.0,
            attack_animation: false,
            walk_frame: 0.0,
            locomotion: None,
            next_animation: 0.0,
            pain_until: 0.0,
            view_offset: Vec3 {
                x: 0.0,
                y: 0.0,
                z: 22.0,
            },
            last_fall_speed: 0.0,
            air_finished: game.time + 12.0,
            drown_damage: 2.0,
            hazard_at: 0.0,
            in_water: false,
        })
    }

    /// Current water level.
    #[must_use]
    pub fn water_level(&self) -> i32 {
        self.input.water_level
    }

    /// Capture checkpoint bytes (`capture`).
    #[must_use]
    pub fn capture(&self) -> Vec<u8> {
        encode_checkpoint_value(&obj(vec![
            ("version", int(3)),
            (
                "input",
                obj(vec![
                    ("axePose", boolean(self.input.axe_pose)),
                    ("attack", boolean(self.input.attack)),
                    ("jump", boolean(self.input.jump)),
                    ("use", boolean(self.input.use_input)),
                    ("waterLevel", int(i64::from(self.input.water_level))),
                    ("waterType", save_str(self.input.water_type.as_str())),
                    ("invisible", boolean(self.input.invisible)),
                    ("invulnerable", boolean(self.input.invulnerable)),
                ]),
            ),
            ("life", save_str(self.life.as_str())),
            ("model", save_str(&self.model)),
            ("modelFrame", num(self.model_frame)),
            (
                "animation",
                self.animation.map_or(crate::value::SaveJson::Null, |animation| {
                    obj(vec![
                        ("first", num(animation.first)),
                        ("count", num(animation.count)),
                        (
                            "end",
                            save_str(match animation.end {
                                AnimationEnd::Locomotion => "locomotion",
                                AnimationEnd::Dead => "dead",
                            }),
                        ),
                    ])
                }),
            ),
            ("animationFrame", num(self.animation_frame)),
            ("attackAnimation", boolean(self.attack_animation)),
            ("walkFrame", num(self.walk_frame)),
            (
                "locomotion",
                self.locomotion.map_or(crate::value::SaveJson::Null, |locomotion| {
                    save_str(match locomotion {
                        Locomotion::Stand => "stand",
                        Locomotion::Run => "run",
                    })
                }),
            ),
            ("nextAnimation", num(self.next_animation)),
            ("painUntil", num(self.pain_until)),
            (
                "viewOffset",
                obj(vec![
                    ("x", num(f64::from(self.view_offset.x))),
                    ("y", num(f64::from(self.view_offset.y))),
                    ("z", num(f64::from(self.view_offset.z))),
                ]),
            ),
            ("lastFallSpeed", num(self.last_fall_speed)),
            ("airFinished", num(self.air_finished)),
            ("drownDamage", num(self.drown_damage)),
            ("hazardAt", num(self.hazard_at)),
            ("inWater", boolean(self.in_water)),
        ]))
    }

    /// Restore checkpoint bytes (`restore`).
    pub fn restore(&mut self, game: &Q1EntityServices, bytes: &[u8]) -> Result<(), Q1Error> {
        let value = decode_checkpoint_value(bytes)?;
        let reader = SaveReader::at(&value, "q1:character");
        let version = reader.field("version").choice_i64(&[1, 2, 3])?;
        let input = reader.field("input");
        self.input = Q1CharacterInput {
            axe_pose: input.field("axePose").boolean()?,
            attack: input.field("attack").boolean()?,
            jump: input.field("jump").boolean()?,
            use_input: input.field("use").boolean()?,
            water_level: i32::try_from(input.field("waterLevel").choice_i64(&[0, 1, 2, 3])?).unwrap_or(0),
            water_type: match input
                .field("waterType")
                .choice_str(&["empty", "water", "slime", "lava"])?
                .as_str()
            {
                "water" => Q1CharacterWater::Water,
                "slime" => Q1CharacterWater::Slime,
                "lava" => Q1CharacterWater::Lava,
                _ => Q1CharacterWater::Empty,
            },
            invisible: input.field("invisible").boolean()?,
            invulnerable: input.field("invulnerable").boolean()?,
        };
        self.life = match reader
            .field("life")
            .choice_str(&["alive", "dying", "dead", "respawnable"])?
            .as_str()
        {
            "dying" => Q1PlayerLife::Dying,
            "dead" => Q1PlayerLife::Dead,
            "respawnable" => Q1PlayerLife::Respawnable,
            _ => Q1PlayerLife::Alive,
        };
        self.model = reader.field("model").string()?;
        self.model_frame = reader.field("modelFrame").number()?;
        self.animation = reader.field("animation").nullable(|value| {
            Ok::<_, Q1Error>(PlayerAnimation {
                first: value.field("first").integer(0)? as f64,
                count: value.field("count").integer(1)? as f64,
                end: match value.field("end").choice_str(&["locomotion", "dead"])?.as_str() {
                    "dead" => AnimationEnd::Dead,
                    _ => AnimationEnd::Locomotion,
                },
            })
        })?;
        self.animation_frame = reader.field("animationFrame").number()?;
        self.attack_animation = reader.field("attackAnimation").boolean()?;
        self.walk_frame = reader.field("walkFrame").number()?;
        self.locomotion = if version == 1 {
            match game.host.bodies.read(self.actor.id()) {
                None => None,
                Some(body) => Some(if body.velocity.x != 0.0 || body.velocity.y != 0.0 {
                    Locomotion::Run
                } else {
                    Locomotion::Stand
                }),
            }
        } else {
            reader.field("locomotion").nullable(|value| {
                Ok::<_, Q1Error>(match value.choice_str(&["stand", "run"])?.as_str() {
                    "run" => Locomotion::Run,
                    _ => Locomotion::Stand,
                })
            })?
        };
        self.next_animation = reader.field("nextAnimation").number()?;
        self.pain_until = reader.field("painUntil").number()?;
        let offset = reader.field("viewOffset");
        self.view_offset = Vec3 {
            x: offset.field("x").number()? as f32,
            y: offset.field("y").number()? as f32,
            z: offset.field("z").number()? as f32,
        };
        self.last_fall_speed = reader.field("lastFallSpeed").number()?;
        self.air_finished = reader.field("airFinished").number()?;
        self.drown_damage = reader.field("drownDamage").number()?;
        self.hazard_at = reader.field("hazardAt").number()?;
        self.in_water = reader.field("inWater").boolean()?;
        if let Some(key) = self.game_key {
            lock_water_table().insert((key, self.actor.id().clone()), self.input.water_level);
        }
        Ok(())
    }

    /// Character lifecycle.
    #[must_use]
    pub fn lifecycle(&self) -> Q1CharacterLifecycle {
        Q1CharacterLifecycle {
            life: self.life,
            view_offset: self.view_offset,
        }
    }

    /// Character presentation.
    #[must_use]
    pub fn presentation(&self) -> Q1CharacterPresentation {
        let pose = if self.life == Q1PlayerLife::Alive {
            self.options.source_pose.map(|probe| probe())
        } else {
            None
        };
        let model = if self.life == Q1PlayerLife::Alive {
            if self.input.invisible {
                String::from("progs/eyes.mdl")
            } else {
                pose.as_ref()
                    .and_then(|pose| pose.definition.as_ref())
                    .map(|definition| definition.model.clone())
                    .unwrap_or_else(|| String::from("progs/player.mdl"))
            }
        } else {
            self.model.clone()
        };
        let frame = if self.life == Q1PlayerLife::Alive {
            if self.input.invisible {
                0.0
            } else {
                pose.as_ref().and_then(|pose| pose.frame).unwrap_or(self.model_frame)
            }
        } else {
            self.model_frame
        };
        Q1CharacterPresentation {
            model,
            frame,
            view_offset: self.view_offset,
            life: self.life,
            solid: if self.life == Q1PlayerLife::Alive {
                Q1Solid::Slidebox
            } else {
                Q1Solid::None
            },
            movement: if self.life == Q1PlayerLife::Alive {
                Q1MoveType::Walk
            } else if self.model == "progs/h_player.mdl" {
                Q1MoveType::Bounce
            } else {
                Q1MoveType::Toss
            },
            weapon_visible: self.life == Q1PlayerLife::Alive,
        }
    }

    fn sound(game: &mut Q1EntityServices, actor: &OwnedActor, path: &str, attenuation: f64) -> Result<(), Q1Error> {
        game.sound(actor.id(), path, Q1SoundChannel::Voice, attenuation, 1.0)
    }

    fn animate(&mut self, game: &Q1EntityServices, animation: PlayerAnimation, attack: bool) {
        self.animation = Some(animation);
        self.attack_animation = attack;
        self.animation_frame = 0.0;
        self.model_frame = animation.first;
        self.next_animation = game.time + 0.1;
    }

    fn select_model(&mut self, game: &Q1EntityServices, pose: Option<&Q1CharacterSourcePose>) {
        if self.life != Q1PlayerLife::Alive {
            return;
        }
        let model = pose
            .and_then(|pose| pose.definition.as_ref())
            .map(|definition| definition.model.clone())
            .unwrap_or_else(|| String::from("progs/player.mdl"));
        if self.model == model {
            return;
        }
        self.model = model;
        self.animation = None;
        self.attack_animation = false;
        self.walk_frame = 0.0;
        self.locomotion = None;
        self.model_frame = pose
            .and_then(|pose| pose.definition.as_ref())
            .map(|definition| definition.stand.first)
            .unwrap_or_else(|| {
                if pose.and_then(|pose| pose.axe_pose).unwrap_or(self.input.axe_pose) {
                    17.0
                } else {
                    12.0
                }
            });
        self.next_animation = game.time;
    }

    /// Run one presentation frame (`frame`).
    pub fn frame(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
        incoming: &Q1CharacterInput,
    ) -> Result<Q1CharacterPresentation, Q1Error> {
        game.time = seconds;
        let pose = self.options.source_pose.map(|probe| probe());
        self.input = Q1CharacterInput {
            axe_pose: pose
                .as_ref()
                .and_then(|pose| pose.axe_pose)
                .unwrap_or(incoming.axe_pose),
            ..incoming.clone()
        };
        if let Some(key) = self.game_key {
            lock_water_table().insert((key, self.actor.id().clone()), self.input.water_level);
        }
        self.select_model(game, pose.as_ref());
        let body = game.host.bodies.read(self.actor.id());
        let source_attack = self.life == Q1PlayerLife::Alive && pose.as_ref().and_then(|pose| pose.frame).is_some();
        if source_attack {
            self.animation = None;
            self.attack_animation = false;
        }
        if (self.life == Q1PlayerLife::Dead || self.life == Q1PlayerLife::Respawnable)
            && body.as_ref().is_some_and(|body| body.ground.is_some())
        {
            if let Some(body) = body.clone() {
                let speed = 0.0f64.max(f64::from(length(body.velocity)) - 20.0);
                game.host.bodies.write(
                    &self.actor,
                    &BodyState {
                        velocity: vscale(normalize(body.velocity), speed),
                        ..body
                    },
                )?;
            }
        }
        if seconds >= self.next_animation && !source_attack {
            self.next_animation = seconds + 0.1;
            if let Some(animation) = self.animation {
                self.animation_frame += 1.0;
                if self.animation_frame < animation.count {
                    self.model_frame = animation.first + self.animation_frame;
                } else {
                    self.animation = None;
                    self.attack_animation = false;
                    if animation.end == AnimationEnd::Dead {
                        self.life = Q1PlayerLife::Dead;
                    }
                }
            }
            if self.animation.is_none() && self.life == Q1PlayerLife::Alive {
                let running = body
                    .as_ref()
                    .is_some_and(|body| body.velocity.x != 0.0 || body.velocity.y != 0.0);
                let locomotion = if running { Locomotion::Run } else { Locomotion::Stand };
                if self.locomotion != Some(locomotion) {
                    self.walk_frame = 0.0;
                }
                self.locomotion = Some(locomotion);
                let definition = pose.as_ref().and_then(|pose| pose.definition.as_ref());
                let frames = match (running, definition) {
                    (true, Some(definition)) => definition.run.clone(),
                    (false, Some(definition)) => definition.stand.clone(),
                    (true, None) => Q1CharacterFrameRange {
                        first: if self.input.axe_pose { 0.0 } else { 6.0 },
                        count: 6.0,
                    },
                    (false, None) if self.input.axe_pose => Q1CharacterFrameRange {
                        first: 17.0,
                        count: 12.0,
                    },
                    (false, None) => Q1CharacterFrameRange {
                        first: 12.0,
                        count: 5.0,
                    },
                };
                self.walk_frame %= frames.count;
                self.model_frame = frames.first + self.walk_frame;
                self.walk_frame += 1.0;
            }
        }
        let pressed = self.input.attack || self.input.jump || self.input.use_input;
        if self.life == Q1PlayerLife::Dead && !pressed {
            self.life = Q1PlayerLife::Respawnable;
        } else if self.life == Q1PlayerLife::Respawnable && pressed {
            if let Some(request) = self.options.request_respawn {
                request();
            }
        }
        Ok(self.presentation())
    }

    /// Present an admitted shot (`attack`).
    pub fn attack(&mut self, game: &Q1EntityServices, attack: Q1CharacterAttack) -> Result<(), Q1Error> {
        if self.life != Q1PlayerLife::Alive {
            return Ok(());
        }
        let pose = self.options.source_pose.map(|probe| probe());
        self.select_model(game, pose.as_ref());
        if pose.as_ref().is_some_and(|pose| pose.definition.is_some())
            || pose.as_ref().and_then(|pose| pose.frame).is_some()
        {
            self.animation = None;
            self.attack_animation = false;
            return Ok(());
        }
        if let Q1CharacterAttack::Axe { variant } = attack {
            self.animate(
                game,
                PlayerAnimation {
                    first: 119.0 + f64::from(variant) * 6.0,
                    count: 6.0,
                    end: AnimationEnd::Locomotion,
                },
                true,
            );
            return Ok(());
        }
        let first = match attack {
            Q1CharacterAttack::Shotgun => 113.0,
            Q1CharacterAttack::Rocket => 107.0,
            Q1CharacterAttack::Nail => 103.0,
            Q1CharacterAttack::Lightning => 105.0,
            Q1CharacterAttack::Axe { .. } => 105.0,
        };
        let count = if matches!(attack, Q1CharacterAttack::Nail | Q1CharacterAttack::Lightning) {
            2.0
        } else {
            6.0
        };
        self.animate(
            game,
            PlayerAnimation {
                first,
                count,
                end: AnimationEnd::Locomotion,
            },
            true,
        );
        Ok(())
    }

    /// React to pain (`pain`).
    pub fn pain(
        &mut self,
        game: &mut Q1EntityServices,
        attacker: Option<&ActorId>,
        _damage: f64,
        axe_hit: bool,
    ) -> Result<(), Q1Error> {
        let pose = self.options.source_pose.map(|probe| probe());
        if self.life != Q1PlayerLife::Alive
            || self.input.invisible
            || self.attack_animation
            || pose.as_ref().and_then(|pose| pose.frame).is_some()
        {
            return Ok(());
        }
        self.select_model(game, pose.as_ref());
        if attacker
            .as_ref()
            .is_some_and(|attacker| game.host.classname(attacker) == "teledeath")
        {
            Self::sound(game, &self.actor.clone(), "player/teledth1.wav", 0.0)?;
        } else if self.input.water_level == 3 && self.input.water_type == Q1CharacterWater::Water {
            self.bubbles(game, 1)?;
            let roll = game.host.random();
            Self::sound(
                game,
                &self.actor.clone(),
                if roll > 0.5 {
                    "player/drown1.wav"
                } else {
                    "player/drown2.wav"
                },
                1.0,
            )?;
        } else if self.input.water_type == Q1CharacterWater::Slime || self.input.water_type == Q1CharacterWater::Lava {
            let roll = game.host.random();
            Self::sound(
                game,
                &self.actor.clone(),
                if roll > 0.5 {
                    "player/lburn1.wav"
                } else {
                    "player/lburn2.wav"
                },
                1.0,
            )?;
        } else if self.pain_until <= game.time {
            self.pain_until = game.time + 0.5;
            let track = (game.host.random() * 5.0 + 1.5).floor() as i32;
            Self::sound(
                game,
                &self.actor.clone(),
                &if axe_hit {
                    String::from("player/axhit1.wav")
                } else {
                    format!("player/pain{track}.wav")
                },
                1.0,
            )?;
        }
        let frames = pose
            .as_ref()
            .and_then(|pose| pose.definition.as_ref())
            .map(|definition| definition.pain.clone())
            .unwrap_or_else(|| Q1CharacterFrameRange {
                first: if pose
                    .as_ref()
                    .and_then(|pose| pose.axe_pose)
                    .unwrap_or(self.input.axe_pose)
                {
                    29.0
                } else {
                    35.0
                },
                count: 6.0,
            });
        self.animate(
            game,
            PlayerAnimation {
                first: frames.first,
                count: frames.count,
                end: AnimationEnd::Locomotion,
            },
            false,
        );
        Ok(())
    }

    /// Die (`die`).
    pub fn die(&mut self, game: &mut Q1EntityServices, attacker: Option<&ActorId>) -> Result<(), Q1Error> {
        if self.life != Q1PlayerLife::Alive {
            return Ok(());
        }
        let body = game.host.bodies.read(self.actor.id());
        let Some(body) = body else { return Ok(()) };
        let pose = self.options.source_pose.map(|probe| probe());
        let definition = pose.as_ref().and_then(|pose| pose.definition.clone());
        self.life = Q1PlayerLife::Dying;
        self.model = definition
            .as_ref()
            .map(|definition| definition.model.clone())
            .unwrap_or_else(|| String::from("progs/player.mdl"));
        self.view_offset = Vec3 {
            x: 0.0,
            y: 0.0,
            z: -8.0,
        };
        if game.health(self.actor.id()) < -99.0 {
            game.host.combat.set_health(&self.actor, -99.0)?;
        }
        let combat = game
            .host
            .combat
            .read(self.actor.id())
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 entity"))?;
        game.host.combat.set_traits(
            &self.actor,
            crate::q1::foundation::gameplay::CombatTraits {
                can_take_damage: false,
                mass: combat.mass,
                invulnerable: combat.invulnerable,
                team: combat.team,
                no_knockback: combat.no_knockback,
            },
        )?;
        if game.player_ref(self.actor.id()).is_some() {
            let powerups: Vec<Q1Powerup> = game
                .player_ref(self.actor.id())
                .map(|player| player.powerups.keys().copied().collect())
                .unwrap_or_default();
            for powerup in powerups {
                game.host.powerup(&self.actor, powerup, 0.0);
            }
            game.update_player(self.actor.id(), |player| player.powerups.clear())?;
        }
        if game.options().deathmatch != 0 || game.options().coop {
            if let Some(drop) = self.options.drop_inventory {
                drop();
            }
        }
        let velocity = if body.velocity.z < 10.0 {
            Vec3 {
                x: body.velocity.x,
                y: body.velocity.y,
                z: body.velocity.z + game.host.random() as f32 * 300.0,
            }
        } else {
            body.velocity
        };
        game.host.bodies.write(
            &self.actor,
            &BodyState {
                velocity,
                ground: None,
                ..body.clone()
            },
        )?;
        let health = game.health(self.actor.id());
        if health < -40.0 {
            self.model = String::from("progs/h_player.mdl");
            self.model_frame = 0.0;
            self.animation = None;
            self.life = Q1PlayerLife::Dead;
            self.view_offset = Vec3 { x: 0.0, y: 0.0, z: 8.0 };
            let scale = if health > -50.0 {
                0.7
            } else if health > -200.0 {
                2.0
            } else {
                10.0
            };
            let (roll_x, roll_y, roll_z) = (
                game.host.random() as f32,
                game.host.random() as f32,
                game.host.random() as f32,
            );
            game.host.bodies.write(
                &self.actor,
                &BodyState {
                    origin: vadd(
                        body.origin,
                        Vec3 {
                            x: 0.0,
                            y: 0.0,
                            z: -24.0,
                        },
                    ),
                    velocity: vscale(
                        Vec3 {
                            x: 100.0 * (roll_x * 2.0 - 1.0),
                            y: 100.0 * (roll_y * 2.0 - 1.0),
                            z: 200.0 + 100.0 * roll_z,
                        },
                        scale,
                    ),
                    bounds: Bounds {
                        min: Vec3 {
                            x: -16.0,
                            y: -16.0,
                            z: 0.0,
                        },
                        max: Vec3 {
                            x: 16.0,
                            y: 16.0,
                            z: 56.0,
                        },
                    },
                    ground: None,
                    ..body.clone()
                },
            )?;
            for model in ["gib1", "gib2", "gib3"] {
                throw_gib(game, body.origin, model, health)?;
            }
            let cause = attacker
                .map(|attacker| game.host.classname(attacker))
                .unwrap_or_default();
            let gib_roll = game.host.random() < 0.5;
            Self::sound(
                game,
                &self.actor.clone(),
                if cause == "teledeath" || cause == "teledeath2" {
                    "player/teledth1.wav"
                } else if gib_roll {
                    "player/gib.wav"
                } else {
                    "player/udeath.wav"
                },
                0.0,
            )?;
            return Ok(());
        }
        if self.input.water_level == 3 {
            self.bubbles(game, 20)?;
            Self::sound(game, &self.actor.clone(), "player/h2odeath.wav", 0.0)?;
        } else {
            let track = (game.host.random() * 4.0 + 1.5).floor() as i32;
            Self::sound(game, &self.actor.clone(), &format!("player/death{track}.wav"), 0.0)?;
        }
        if let Some(current) = game.host.bodies.read(self.actor.id()) {
            game.host.bodies.write(
                &self.actor,
                &BodyState {
                    angles: Vec3 {
                        x: 0.0,
                        y: current.angles.y,
                        z: 0.0,
                    },
                    ..current
                },
            )?;
        }
        let animation = if let Some(definition) = definition {
            PlayerAnimation {
                first: definition.death.first,
                count: definition.death.count,
                end: AnimationEnd::Dead,
            }
        } else if pose
            .as_ref()
            .and_then(|pose| pose.axe_pose)
            .unwrap_or(self.input.axe_pose)
        {
            PlayerAnimation {
                first: 41.0,
                count: 9.0,
                end: AnimationEnd::Dead,
            }
        } else {
            *DEATHS
                .get((game.host.random() * 6.0) as usize)
                .ok_or_else(|| crate::q1::q1_error("Player death animation selection outside source random range"))?
        };
        self.animate(game, animation, false);
        Ok(())
    }

    /// Respawn (`respawn`).
    pub fn respawn(&mut self, game: &mut Q1EntityServices, health: Option<f64>) -> Result<(), Q1Error> {
        self.life = Q1PlayerLife::Alive;
        self.model = String::from("progs/player.mdl");
        self.model_frame = 12.0;
        self.animation = None;
        self.attack_animation = false;
        self.walk_frame = 0.0;
        self.locomotion = None;
        self.pain_until = 0.0;
        self.view_offset = Vec3 {
            x: 0.0,
            y: 0.0,
            z: 22.0,
        };
        self.air_finished = game.time + 12.0;
        self.drown_damage = 2.0;
        self.hazard_at = 0.0;
        self.in_water = false;
        self.last_fall_speed = 0.0;
        let pose = self.options.source_pose.map(|probe| probe());
        self.select_model(game, pose.as_ref());
        let combat = game
            .host
            .combat
            .read(self.actor.id())
            .ok_or_else(|| crate::q1::q1_error("Missing Q1 entity"))?;
        game.host.combat.set_traits(
            &self.actor,
            crate::q1::foundation::gameplay::CombatTraits {
                can_take_damage: true,
                mass: combat.mass,
                invulnerable: combat.invulnerable,
                team: combat.team,
                no_knockback: combat.no_knockback,
            },
        )?;
        if let Some(health) = health {
            game.host.combat.set_health(&self.actor, health)?;
        }
        Ok(())
    }

    /// Source kill/disconnect pose (`setSuicideFrame`).
    pub fn set_suicide_frame(&mut self) {
        self.model_frame = 60.0;
        self.life = Q1PlayerLife::Dead;
        self.animation = None;
        self.attack_animation = false;
        self.next_animation = f64::INFINITY;
    }

    fn bubbles(&mut self, game: &mut Q1EntityServices, count: i32) -> Result<(), Q1Error> {
        let timer = game.create("death_bubbles", None, None)?;
        game.update_entity(&timer, |entity| {
            entity.owner = Some(self.actor.id().clone());
            entity.count = f64::from(count);
        })?;
        game.schedule(&timer, 0.1, "base:death_bubbles")
    }

    /// Apply Q1 landing rules after movement (`postMove`).
    pub fn post_move(&mut self, game: &mut Q1EntityServices) -> Result<(), Q1Error> {
        if self.life != Q1PlayerLife::Alive {
            return Ok(());
        }
        let body = game.host.bodies.read(self.actor.id());
        let Some(body) = body else { return Ok(()) };
        if self.last_fall_speed < -300.0 && body.ground.is_some() && game.health(self.actor.id()) > 0.0 {
            if self.input.water_type == Q1CharacterWater::Water {
                game.sound(self.actor.id(), "player/h2ojump.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
            } else if self.last_fall_speed < -650.0 && self.options.fall_damage_allowed.is_none_or(|allowed| allowed())
            {
                let world = game.world.clone();
                game.damage(
                    self.actor.id(),
                    Some(world.as_ref().unwrap_or(self.actor.id())),
                    world.as_ref(),
                    5.0,
                    &Q1DamageParams {
                        death_type: String::from("falling"),
                        ..Default::default()
                    },
                );
                Self::sound(game, &self.actor.clone(), "player/land2.wav", 1.0)?;
            } else {
                Self::sound(game, &self.actor.clone(), "player/land.wav", 1.0)?;
            }
            self.last_fall_speed = 0.0;
        }
        if body.ground.is_none() {
            self.last_fall_speed = f64::from(body.velocity.z);
        }
        Ok(())
    }

    /// Run environmental rules once per frame (`environment`).
    pub fn environment(
        &mut self,
        game: &mut Q1EntityServices,
        seconds: f64,
        suit: bool,
        noclip: bool,
    ) -> Result<(), Q1Error> {
        if self.life != Q1PlayerLife::Alive || noclip {
            return Ok(());
        }
        game.time = seconds;
        let lava_suit = game
            .player_ref(self.actor.id())
            .and_then(|player| player.powerups.get(&Q1Powerup::Mg3Lavasuit).copied())
            .unwrap_or(0.0)
            > seconds;
        if self.input.water_level != 3 {
            if self.air_finished < seconds {
                Self::sound(game, &self.actor.clone(), "player/gasp2.wav", 1.0)?;
            } else if self.air_finished < seconds + 9.0 {
                Self::sound(game, &self.actor.clone(), "player/gasp1.wav", 1.0)?;
            }
            self.air_finished = seconds + 12.0;
            self.drown_damage = 2.0;
        } else if suit || lava_suit {
            self.air_finished = seconds + 12.0;
        } else if self.air_finished < seconds && self.pain_until < seconds {
            self.drown_damage += 2.0;
            if self.drown_damage > 15.0 {
                self.drown_damage = 10.0;
            }
            let world = game.world.clone();
            game.damage(
                self.actor.id(),
                Some(world.as_ref().unwrap_or(self.actor.id())),
                world.as_ref(),
                self.drown_damage,
                &Q1DamageParams {
                    death_type: String::from("drown"),
                    ..Default::default()
                },
            );
            self.pain_until = seconds + 1.0;
        }
        if self.input.water_level == 0 {
            if self.in_water {
                game.sound(self.actor.id(), "misc/outwater.wav", Q1SoundChannel::Body, 1.0, 1.0)?;
            }
            self.in_water = false;
            return Ok(());
        }
        if self.hazard_at < seconds && self.input.water_type == Q1CharacterWater::Lava && !lava_suit {
            self.hazard_at = seconds + if suit { 1.0 } else { 0.2 };
            let world = game.world.clone();
            game.damage(
                self.actor.id(),
                Some(world.as_ref().unwrap_or(self.actor.id())),
                world.as_ref(),
                10.0 * f64::from(self.input.water_level),
                &Q1DamageParams {
                    death_type: String::from("lava"),
                    ..Default::default()
                },
            );
        } else if self.hazard_at < seconds && self.input.water_type == Q1CharacterWater::Slime && !suit && !lava_suit {
            self.hazard_at = seconds + 1.0;
            let world = game.world.clone();
            game.damage(
                self.actor.id(),
                Some(world.as_ref().unwrap_or(self.actor.id())),
                world.as_ref(),
                4.0 * f64::from(self.input.water_level),
                &Q1DamageParams {
                    death_type: String::from("slime"),
                    ..Default::default()
                },
            );
        }
        if !self.in_water {
            game.sound(
                self.actor.id(),
                if self.input.water_type == Q1CharacterWater::Lava {
                    "player/inlava.wav"
                } else if self.input.water_type == Q1CharacterWater::Slime {
                    "player/slimbrn2.wav"
                } else {
                    "player/inh2o.wav"
                },
                Q1SoundChannel::Body,
                1.0,
                1.0,
            )?;
            self.in_water = true;
            self.hazard_at = 0.0;
        }
        Ok(())
    }
}

fn death_bubbles(game: &mut Q1EntityServices, id: &ActorId) -> Result<(), Q1Error> {
    let id = id.clone();
    let owner = game.entity_ref(&id).and_then(|entity| entity.owner.clone());
    let owner = owner.and_then(|owner| game.host.actors.resolve_owned(&owner));
    let Some(owner) = owner else { return game.remove(&id) };
    let level = lock_water_table()
        .get(&(game_key(game), owner.id().clone()))
        .copied()
        .or_else(|| game.player_ref(owner.id()).map(|player| player.water_level))
        .unwrap_or(0);
    if level != 3 {
        return Ok(());
    }
    let body = game.host.bodies.read(owner.id());
    let Some(body) = body else { return game.remove(&id) };
    spawn_bubble(
        game,
        vadd(
            body.origin,
            Vec3 {
                x: 0.0,
                y: 0.0,
                z: 24.0,
            },
        ),
        Vec3 {
            x: 0.0,
            y: 0.0,
            z: 15.0,
        },
        false,
    )?;
    game.update_entity(&id, |entity| entity.count -= 1.0)?;
    if game.entity_ref(&id).map(|entity| entity.count).unwrap_or(0.0) <= 0.0 {
        return game.remove(&id);
    }
    game.schedule(&id, 0.1, "base:death_bubbles")
}

/// Character actor-release cleanup.
struct Q1CharacterReleaseHook;

impl Q1ReleaseHook for Q1CharacterReleaseHook {
    fn on_release(&mut self, game: &mut Q1EntityServices, actor: &OwnedActor) {
        lock_water_table().remove(&(game_key(game), actor.id().clone()));
    }
}

/// Register character callbacks (`registerCharacterCallbacks`).
pub fn register_character_callbacks(game: &mut Q1EntityServices) -> Result<(), Q1Error> {
    game.named.register(
        "base:death_bubbles",
        crate::q1::foundation::callbacks::Q1CallbackHandlers {
            action: Some(death_bubbles),
            ..Default::default()
        },
    )?;
    game.register_release_hook(Rc::new(RefCell::new(Q1CharacterReleaseHook)));
    Ok(())
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;
    use crate::q1::base::provider::{Q1BaseGuard, Q1BaseOptions};
    use crate::q1::foundation::entity_services::Q1AttachOptions;
    use crate::q1::foundation::host::mock::mock_host;
    use crate::q1::foundation::types::{Q1Edition, Q1FoundationOptions, Q1PrecacheProgram};

    fn options() -> Q1FoundationOptions {
        Q1FoundationOptions {
            provider: None,
            precache_program: Some(Q1PrecacheProgram::Id1),
            edition: Q1Edition::Classic,
            physics_edition: None,
            skill: 1,
            deathmatch: 0,
            coop: false,
            campaign: ProviderId::new("q1", "campaign"),
            combat_provider: ProviderId::new("q1", "combat"),
            movement_provider: ProviderId::new("q1", "movement"),
            inventory_provider: ProviderId::new("q1", "inventory"),
            gravity: 800.0,
            max_clients: Some(4),
            no_exit: None,
            teamplay: None,
            aim_threshold: None,
        }
    }

    fn character(game: &mut Q1EntityServices) -> Q1CharacterActor {
        let player = game.create("player", None, None).expect("player");
        let owned = game
            .entity_ref(&player)
            .map(|entity| entity.actor.clone())
            .expect("owned");
        game.attach_player(&owned, &Q1AttachOptions::default()).expect("attach");
        Q1CharacterActor::new(game, owned, Q1CharacterOptions::default()).expect("character")
    }

    #[test]
    fn frame_advances_locomotion_and_attacks() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let mut hero = character(&mut game);
        let input = Q1CharacterInput::default();
        let first = hero.frame(&mut game, 0.1, &input).expect("frame");
        assert_eq!(first.frame, 12.0);
        let second = hero.frame(&mut game, 0.2, &input).expect("frame");
        assert_eq!(second.frame, 13.0);
        hero.attack(&game, Q1CharacterAttack::Shotgun).expect("attack");
        assert_eq!(hero.presentation().frame, 113.0);
        hero.set_suicide_frame();
        assert_eq!(hero.presentation().life, Q1PlayerLife::Dead);
    }

    #[test]
    fn death_gibs_and_checkpoint_round_trips() {
        let (host, _) = mock_host();
        let mut game = Q1EntityServices::new(host, options()).expect("game");
        let _guard = Q1BaseGuard::register(&mut game, Q1BaseOptions::default()).expect("base");
        let mut hero = character(&mut game);
        game.host.combat.set_health(&hero.actor, -50.0).expect("health");
        hero.die(&mut game, None).expect("die");
        assert_eq!(hero.presentation().model, "progs/h_player.mdl");
        assert_eq!(hero.presentation().movement, Q1MoveType::Bounce);
        let bytes = hero.capture();
        hero.respawn(&mut game, Some(100.0)).expect("respawn");
        assert_eq!(hero.presentation().life, Q1PlayerLife::Alive);
        hero.restore(&game, &bytes).expect("restore");
        assert_eq!(hero.presentation().life, Q1PlayerLife::Dead);
        hero.environment(&mut game, 1.0, false, false).expect("environment");
        hero.post_move(&mut game).expect("posture");
    }
}
