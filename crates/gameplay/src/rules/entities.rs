//! Source-native meanings of overloaded entity cells and published trajectories.
use qa_core::primitives::{CollisionTags, RuleSetId, ThinkTime, Trajectory, TrajectoryCurve, Vec3};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelCellRole {
    Models,
    Item,
    Team,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ParameterRole {
    Scalar,
    Sound,
    Entity,
}
#[derive(Clone, Copy)]
pub struct EntityCellRoles {
    pub models: ModelCellRole,
    pub parameter: ParameterRole,
    pub client_reference: bool,
    pub other_reference: bool,
    pub other2_reference: bool,
}

#[derive(Clone, Copy)]
pub struct EntityRoles {
    event_base: Option<u32>,
    event_mask: u32,
    item: Option<u32>,
    team: Option<u32>,
    player: Option<u32>,
    speaker: Option<u32>,
    sounds: &'static [u32],
    parameter_entities: &'static [u32],
    client_events: &'static [u32],
    other_events: &'static [u32],
    other2_events: &'static [u32],
    player_event_flag: u32,
    client_non_solids: &'static [u32],
}
const NEUTRAL: EntityRoles = EntityRoles {
    event_base: None,
    event_mask: u32::MAX,
    item: None,
    team: None,
    player: None,
    speaker: None,
    sounds: &[],
    parameter_entities: &[],
    client_events: &[],
    other_events: &[],
    other2_events: &[],
    player_event_flag: 0,
    client_non_solids: &[],
};
const Q3: EntityRoles = EntityRoles {
    event_base: Some(13),
    event_mask: !0x300,
    item: Some(2),
    team: Some(12),
    player: Some(1),
    speaker: Some(7),
    sounds: &[45, 46],
    parameter_entities: &[48],
    client_events: &[1, 2, 3, 4, 5, 6, 53],
    other_events: &[48, 49, 50, 54, 60],
    other2_events: &[60],
    player_event_flag: 0x10,
    client_non_solids: &[2, 8, 9],
};
pub const fn entity_roles(source: RuleSetId) -> EntityRoles {
    [NEUTRAL, NEUTRAL, NEUTRAL, NEUTRAL, Q3][source as usize]
}
impl EntityRoles {
    pub fn project_type(self, native: Option<u32>, tags: CollisionTags) -> u32 {
        native.unwrap_or_else(|| {
            self.player
                .filter(|_| tags.0 & CollisionTags::PLAYER.0 != 0)
                .unwrap_or(0)
        })
    }

    pub fn is_player(self, kind: u32) -> bool {
        self.player == Some(kind)
    }

    pub fn client_solid(self, kind: u32) -> bool {
        !self.client_non_solids.contains(&kind)
    }

    pub fn cells(self, entity_type: u32, event: u32, flags: u32) -> EntityCellRoles {
        let standalone = self.event_base.is_some_and(|base| entity_type > base);
        let event = self
            .event_base
            .filter(|_| standalone)
            .map_or(event, |base| entity_type - base)
            & self.event_mask;
        EntityCellRoles {
            models: if self.item == Some(entity_type) {
                ModelCellRole::Item
            } else if self.team == Some(entity_type) {
                ModelCellRole::Team
            } else {
                ModelCellRole::Models
            },
            parameter: if self.speaker == Some(entity_type) || self.sounds.contains(&event) {
                ParameterRole::Sound
            } else if self.parameter_entities.contains(&event) {
                ParameterRole::Entity
            } else {
                ParameterRole::Scalar
            },
            client_reference: self.is_player(entity_type) || self.client_events.contains(&event),
            other_reference: self.other_events.contains(&event)
                || standalone && flags & self.player_event_flag != 0,
            other2_reference: self.other2_events.contains(&event),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrajectorySample {
    Value,
    Delta,
}

#[derive(Clone, Copy)]
pub struct TrajectoryRules {
    binary32: bool,
    gravity: f32,
    client_limit: u32,
    mover_kind: u32,
    world_number: u32,
}
impl TrajectoryRules {
    pub const Q3_NATIVE: Self = Self {
        binary32: false,
        gravity: 800.0,
        client_limit: 64,
        mover_kind: 4,
        world_number: 1022,
    };
    pub const Q3_QVM: Self = Self {
        binary32: true,
        ..Self::Q3_NATIVE
    };
    pub fn position_curve(
        self,
        curve: TrajectoryCurve,
        number: u32,
        smooth_clients: bool,
    ) -> TrajectoryCurve {
        if number < self.client_limit && !smooth_clients {
            TrajectoryCurve::Interpolate
        } else {
            curve
        }
    }
    pub fn interpolates(self, curve: TrajectoryCurve, number: u32) -> bool {
        curve == TrajectoryCurve::Interpolate
            || curve == TrajectoryCurve::LinearStop && number < self.client_limit
    }
    pub fn is_mover(self, number: u32, kind: u32) -> bool {
        number > 0 && number < self.world_number && kind == self.mover_kind
    }

    /// Source clocks stay signed32. Queued or shader clocks never supply a
    /// native trajectory time without the caller's accepted clock projection.
    pub fn sample(
        self,
        trajectory: &Trajectory,
        at: ThinkTime,
        sample: TrajectorySample,
    ) -> Option<Vec3> {
        let (ThinkTime::Milliseconds(at), ThinkTime::Milliseconds(start)) = (at, trajectory.start)
        else {
            return None;
        };
        let at = at as i32;
        let start = start as i32;
        let duration = trajectory.duration_ms as i32;
        let delta = matches!(sample, TrajectorySample::Delta);
        let mut elapsed = at.wrapping_sub(start);
        let scale = match trajectory.curve {
            TrajectoryCurve::Stationary | TrajectoryCurve::Interpolate => {
                let result = if delta {
                    Vec3::default()
                } else {
                    trajectory.base
                };
                return result.is_finite().then_some(result);
            }
            TrajectoryCurve::Linear => {
                if delta {
                    return Some(trajectory.delta);
                }
                self.seconds(elapsed)
            }
            TrajectoryCurve::LinearStop => {
                let end = start.wrapping_add(duration);
                if delta {
                    return Some(if at > end {
                        Vec3::default()
                    } else {
                        trajectory.delta
                    });
                }
                if at > end {
                    elapsed = end.wrapping_sub(start);
                }
                self.seconds(elapsed).max(0.0)
            }
            TrajectoryCurve::Sine => {
                if duration == 0 {
                    return None;
                }
                let fraction = elapsed as f32 / duration as f32;
                let radians = if self.binary32 {
                    (fraction * std::f32::consts::PI * 2.0) as f64
                } else {
                    f64::from(fraction) * std::f64::consts::PI * 2.0
                };
                if delta {
                    (radians.cos() as f32) * 0.5
                } else {
                    radians.sin() as f32
                }
            }
            TrajectoryCurve::Gravity => {
                let seconds = self.seconds(elapsed);
                let mut result = if delta {
                    trajectory.delta
                } else {
                    trajectory.base + trajectory.delta * seconds
                };
                result.0[2] = if delta {
                    result.0[2] - self.gravity * seconds
                } else if self.binary32 {
                    result.0[2] - ((0.5 * self.gravity) * seconds) * seconds
                } else {
                    (f64::from(result.0[2])
                        - 0.5 * f64::from(self.gravity) * f64::from(seconds) * f64::from(seconds))
                        as f32
                };
                return result.is_finite().then_some(result);
            }
            TrajectoryCurve::Unsupported(_) => return None,
        };
        let result = if delta {
            trajectory.delta * scale
        } else {
            trajectory.base + trajectory.delta * scale
        };
        result.is_finite().then_some(result)
    }
    fn seconds(self, milliseconds: i32) -> f32 {
        if self.binary32 {
            milliseconds as f32 * 0.001f32
        } else {
            (f64::from(milliseconds) * 0.001) as f32
        }
    }
}
