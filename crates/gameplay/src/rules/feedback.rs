//! Native player damage becomes a shared camera response at its source boundary.
use qa_core::{
    math::{angle_vectors_radians, length, radians_from_degrees_f32},
    primitives::{
        ClockObservation, PlayerDamageSample, PlayerSignal, RuleSetId, SoundBus, SoundPlayback,
        ThinkTime, Vec3, ViewFeedback,
    },
};

#[derive(Clone, Copy)]
pub struct PlayerEventRules {
    toggle_mask: u32,
    sounds: &'static [Option<PlayerSoundCue>; 64],
    pub playback: Option<SoundPlayback>,
}

/// File-backed media and native configstring ordinals are different domains.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PlayerSoundBinding {
    Config(u32),
    Fixed(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerSoundCue {
    pub binding: PlayerSoundBinding,
    pub channel: i32,
    pub bus: SoundBus,
}

/// CG_RegisterSounds media used by the immediate, context-free event recipes.
pub const PLAYER_SOUND_PATHS: [&str; 13] = [
    "sound/player/land1.wav",
    "sound/player/watr_in.wav",
    "sound/player/watr_out.wav",
    "sound/player/watr_un.wav",
    "sound/weapons/change.wav",
    "sound/items/use_nothing.wav",
    "sound/items/use_medkit.wav",
    "sound/items/respawn1.wav",
    "sound/world/telein.wav",
    "sound/world/teleout.wav",
    "sound/items/damage3.wav",
    "sound/items/protect3.wav",
    "sound/items/regen.wav",
];

const fn q3_player_sounds() -> [Option<PlayerSoundCue>; 64] {
    let mut sounds = [None; 64];
    // CG_EntityEvent native channels: AUTO=0, ITEM=4, BODY=5.
    let fixed = [
        (10, 0, 0),
        (15, 1, 0),
        (16, 2, 0),
        (17, 3, 0),
        (22, 4, 0),
        (24, 5, 5),
        (26, 6, 5),
        (40, 7, 0),
        (41, 7, 0),
        (42, 8, 0),
        (43, 9, 0),
        (61, 10, 4),
        (62, 11, 4),
        (63, 12, 4),
    ];
    let mut index = 0;
    while index < fixed.len() {
        let (event, media, channel) = fixed[index];
        sounds[event] = Some(PlayerSoundCue {
            binding: PlayerSoundBinding::Fixed(media),
            channel,
            bus: SoundBus::Spatial,
        });
        index += 1;
    }
    // HI_NUM_HOLDABLE=6. CG_UseItem uses > rather than >=, then its
    // default switch case handles the tie as well. BASE/TA use3..5 differ.
    let mut event = 30;
    while event <= 38 {
        sounds[event] = sounds[24];
        event += 1;
    }
    sounds[45] = Some(PlayerSoundCue {
        binding: PlayerSoundBinding::Config(0),
        channel: 3,
        bus: SoundBus::Spatial,
    });
    sounds[46] = Some(PlayerSoundCue {
        binding: PlayerSoundBinding::Config(0),
        channel: 0,
        bus: SoundBus::Local,
    });
    sounds
}

const Q3_PLAYER_SOUNDS: [Option<PlayerSoundCue>; 64] = q3_player_sounds();

pub const fn player_event_rules(source: RuleSetId) -> Option<PlayerEventRules> {
    // Source capability, independently of the map, movement and sound role.
    [
        None,
        None,
        None,
        None,
        Some(PlayerEventRules {
            toggle_mask: 0x300,
            sounds: &Q3_PLAYER_SOUNDS,
            playback: super::native_sound_playback(RuleSetId::Quake3),
        }),
    ][source as usize]
}

impl PlayerEventRules {
    pub fn code(self, code: u32) -> u32 {
        code & !self.toggle_mask
    }
    pub fn sound(self, signal: PlayerSignal) -> Option<PlayerSoundCue> {
        let code = self.code(signal.code);
        let mut cue = self.sounds.get(code as usize).copied().flatten()?;
        if matches!(cue.binding, PlayerSoundBinding::Config(_)) {
            cue.binding = PlayerSoundBinding::Config(signal.parameter);
        }
        Some(cue)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct DamageFeedbackRules {
    health_threshold: i32,
    health_scale: f64,
    kick_range: [f32; 2],
    centered: u32,
    direction_units: f64,
    deflect_ms: f32,
    return_ms: f64,
}

pub fn damage_feedback_rules(source: RuleSetId) -> Option<DamageFeedbackRules> {
    // CG_DamageFeedback and CG_OffsetFirstPersonView, cg_playerstate.c/cg_view.c.
    (source == RuleSetId::Quake3).then_some(DamageFeedbackRules {
        health_threshold: 40,
        health_scale: 40.0,
        kick_range: [5.0, 10.0],
        centered: 255,
        direction_units: 255.0,
        deflect_ms: 100.0,
        return_ms: 400.0,
    })
}

impl DamageFeedbackRules {
    /// Axes are the previous submitted view axes, including its camera kick.
    /// Dead-view and native time-zero suppression belong to the view consumer.
    pub fn project(
        self,
        sample: PlayerDamageSample,
        health: i32,
        prior_view_axes: [Vec3; 3],
        clock: ClockObservation,
    ) -> Option<ViewFeedback> {
        if sample.count == 0 || !matches!(clock.native, ThinkTime::Milliseconds(_)) {
            return None;
        }
        let scale = if health < self.health_threshold {
            1.0
        } else {
            (self.health_scale / f64::from(health)) as f32
        };
        let kick =
            (sample.count as i32 as f32 * scale).clamp(self.kick_range[0], self.kick_range[1]);
        let (angles, screen) = if sample.yaw == self.centered && sample.pitch == self.centered {
            (Vec3([-kick, 0.0, 0.0]), [0.0, 0.0])
        } else {
            let pitch = (f64::from(sample.pitch as i32) / self.direction_units * 360.0) as f32;
            let yaw = (f64::from(sample.yaw as i32) / self.direction_units * 360.0) as f32;
            let direction = Vec3::default()
                - angle_vectors_radians(radians_from_degrees_f32(Vec3([pitch, yaw, 0.0]))).forward;
            let front = direction.dot(prior_view_axes[0]);
            let left = direction.dot(prior_view_axes[1]);
            let up = direction.dot(prior_view_axes[2]);
            let mut distance = length(Vec3([front, left, 0.0]));
            if f64::from(distance) < 0.1 {
                distance = 0.1;
            }
            let angles = Vec3([-kick * front, 0.0, kick * left]);
            let mut front = front;
            if f64::from(front) <= 0.1 {
                front = 0.1;
            }
            let screen = [
                (-left / front).clamp(-1.0, 1.0),
                (up / distance).clamp(-1.0, 1.0),
            ];
            (angles, screen)
        };
        Some(ViewFeedback {
            kick: angles,
            screen,
            value: kick,
            clock,
            deflect_ms: self.deflect_ms,
            return_ms: self.return_ms,
        })
    }
}
