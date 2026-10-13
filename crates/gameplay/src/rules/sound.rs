use qa_core::primitives::{
    EntityAppearance, LoopGain, LoopLifetime, LoopOrigin, LoopPhase, LoopPlayback,
    NativeAppearanceTail, RuleSetId, SoundChannels, SoundPlayback, SpatialPolicy,
};

const Q1: SpatialPolicy = SpatialPolicy {
    distance_offset: 0.0,
    distance_scale: 0.001,
    separation: 1.0,
};
const Q2: SpatialPolicy = SpatialPolicy {
    distance_offset: 80.0,
    distance_scale: 0.001,
    separation: 0.5,
};
const Q3: SpatialPolicy = SpatialPolicy {
    distance_offset: 80.0,
    distance_scale: 1.0 / (1250.0 - 80.0),
    separation: 0.5,
};
const POLICIES: [SpatialPolicy; 5] = [Q1, Q1, Q2, Q2, Q3];

pub const fn sound_policy(rules: RuleSetId) -> SpatialPolicy {
    POLICIES[rules as usize]
}

const Q2_LOOP_SPATIAL: SpatialPolicy = SpatialPolicy {
    distance_scale: 0.003,
    ..Q2
};
const Q3_LOOP_SPATIAL: SpatialPolicy = SpatialPolicy {
    distance_scale: 0.0008,
    ..Q3
};
const Q3_PLAYBACK: SoundPlayback = SoundPlayback {
    master: 127,
    volume_scale: 255,
    clamp_pan: true,
    latched: true,
    channels: Some(SoundChannels {
        suppress_ms: 50,
        emitter_limit: 4,
        listener_limit: 8,
        protected_channel: 7,
    }),
};
/// Native CLIENT imports use the original DMA spatial/gain capabilities.
pub const fn native_sound_playback(rules: RuleSetId) -> Option<SoundPlayback> {
    [None, None, None, None, Some(Q3_PLAYBACK)][rules as usize]
}
pub const fn native_sound_spatial(rules: RuleSetId) -> SpatialPolicy {
    [Q1, Q1, Q2, Q2, Q3_LOOP_SPATIAL][rules as usize]
}
const LOOP_SPATIAL: [SpatialPolicy; 5] =
    [Q1, Q1, Q2_LOOP_SPATIAL, Q2_LOOP_SPATIAL, Q3_LOOP_SPATIAL];

/// Presentation is selected independently of the received native fields.
pub const fn loop_spatial_policy(rules: RuleSetId) -> SpatialPolicy {
    LOOP_SPATIAL[rules as usize]
}

const Q2_LOOP: LoopPlayback = LoopPlayback {
    master: LoopGain::Integer {
        master: 255,
        volume_scale: 256,
    },
    phase: LoopPhase::Painted,
    lifetime: LoopLifetime::Temporary,
    panning: true,
    clamp_pan: false,
    origin: LoopOrigin::Accepted,
    latched: false,
};
const RR_LOOP: LoopPlayback = LoopPlayback {
    master: LoopGain::Float(1.0),
    origin: LoopOrigin::Presented,
    ..Q2_LOOP
};
const Q3_LOOP: LoopPlayback = LoopPlayback {
    master: LoopGain::Integer {
        master: 127,
        volume_scale: 255,
    },
    origin: LoopOrigin::Presented,
    clamp_pan: true,
    ..Q2_LOOP
};
const Q3_REAL: LoopPlayback = LoopPlayback {
    master: LoopGain::Integer {
        master: 90,
        volume_scale: 255,
    },
    lifetime: LoopLifetime::Persistent,
    ..Q3_LOOP
};

/// Source field interpretation is cached at the CLIENT connection boundary.
#[derive(Clone, Copy)]
pub struct LoopRules {
    ordinary: LoopPlayback,
    speaker: Option<(u32, LoopPlayback)>,
    event_base: Option<u32>,
    stop_event: Option<(u32, u32, u32)>,
    no_stereo_mask: u32,
    spatial: fn(f32, SpatialPolicy) -> SpatialPolicy,
}
impl LoopRules {
    /// Direct CLIENT imports select temporary/real capability data without
    /// manufacturing an entity appearance or inferring another game role.
    pub fn native(self, persistent: bool) -> LoopPlayback {
        let mut playback = if persistent {
            self.speaker.map_or(self.ordinary, |(_, playback)| playback)
        } else {
            self.ordinary
        };
        playback.latched = true;
        playback
    }
    /// Standalone native events fire once while the accepted entity remains.
    pub fn event_once(self, native: &NativeAppearanceTail) -> bool {
        self.event_base
            .is_some_and(|first| native.entity_type > first)
    }
    pub fn admit(
        self,
        appearance: &EntityAppearance,
        spatial: SpatialPolicy,
    ) -> Option<(LoopPlayback, SpatialPolicy)> {
        appearance.loop_sound?;
        let native = &appearance.native;
        if self
            .event_base
            .is_some_and(|first| native.entity_type >= first)
        {
            return None;
        }
        let mut playback = self
            .speaker
            .filter(|(kind, _)| native.entity_type == *kind)
            .map_or(self.ordinary, |(_, playback)| playback);
        if let LoopGain::Float(ref mut master) = playback.master {
            *master = if native.loop_volume == 0.0 {
                1.0
            } else {
                native.loop_volume
            };
        }
        playback.panning &= native.render_flags & self.no_stereo_mask == 0;
        Some((playback, (self.spatial)(native.loop_attenuation, spatial)))
    }

    /// The native toggle bits remain in the producer's event key.
    pub fn stop(self, native: &NativeAppearanceTail) -> Option<u32> {
        let (mask, value, _) = self.stop_event?;
        let (event, key) = match self.event_base {
            Some(first) if native.entity_type > first => (
                native.entity_type.checked_sub(first)?,
                0x8000_0000 | native.entity_type,
            ),
            _ => (native.event, native.event),
        };
        (event & mask == value).then_some(key)
    }

    /// A standalone player event carries another native entity's number.
    /// SERVER maps it in that module's namespace before the wire projection.
    pub fn stop_target(self, native: &NativeAppearanceTail) -> Option<u32> {
        self.stop(native)?;
        let (_, _, player_event) = self.stop_event?;
        if self.event_once(native) && native.render_flags & player_event != 0 {
            Some(native.other_entity)
        } else {
            None
        }
    }

    pub fn stop_emitter(self, appearance: &EntityAppearance, native_number: u32) -> Option<u32> {
        self.stop(&appearance.native)?;
        Some(
            self.stop_target(&appearance.native)
                .unwrap_or(native_number),
        )
    }
}
fn keep_spatial(_: f32, spatial: SpatialPolicy) -> SpatialPolicy {
    spatial
}
fn loop_attenuation(value: f32, mut spatial: SpatialPolicy) -> SpatialPolicy {
    if value == -1.0 {
        spatial.distance_scale = 0.0;
    } else if value > 0.0 && value != 3.0 {
        spatial.distance_scale = value * 0.0006;
    }
    spatial
}
const LOOP_RULES: [Option<LoopRules>; 5] = [
    None,
    None,
    Some(LoopRules {
        ordinary: Q2_LOOP,
        speaker: None,
        event_base: None,
        stop_event: None,
        no_stereo_mask: 0,
        spatial: keep_spatial,
    }),
    Some(LoopRules {
        ordinary: RR_LOOP,
        speaker: None,
        event_base: None,
        stop_event: None,
        no_stereo_mask: 4,
        spatial: loop_attenuation,
    }),
    Some(LoopRules {
        ordinary: Q3_LOOP,
        speaker: Some((7, Q3_REAL)),
        event_base: Some(13),
        stop_event: Some((!0x300, 75, 0x10)),
        no_stereo_mask: 0,
        spatial: keep_spatial,
    }),
];

pub const fn loop_rules(source: RuleSetId) -> Option<LoopRules> {
    LOOP_RULES[source as usize]
}
