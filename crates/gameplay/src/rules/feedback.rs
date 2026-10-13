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
    sounds: [(u32, i32, SoundBus); 2],
    pub playback: Option<SoundPlayback>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlayerSoundCue {
    pub ordinal: u32,
    pub channel: i32,
    pub bus: SoundBus,
}

pub const fn player_event_rules(source: RuleSetId) -> Option<PlayerEventRules> {
    // CG_EntityEvent, EV_GENERAL_SOUND/EV_GLOBAL_SOUND. Native channel numbers
    // differ from Q2; the global event plays from this recipient's head.
    [
        None,
        None,
        None,
        None,
        Some(PlayerEventRules {
            toggle_mask: 0x300,
            sounds: [(45, 3, SoundBus::Spatial), (46, 0, SoundBus::Local)],
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
        let &(_, channel, bus) = self.sounds.iter().find(|&&(event, _, _)| event == code)?;
        Some(PlayerSoundCue {
            ordinal: signal.parameter,
            channel,
            bus,
        })
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
