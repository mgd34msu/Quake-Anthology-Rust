//! Bot entity observations from `src/bots/behavior/q3/observations.ts`
//! (`aas_entityinfo_t`).
//!
//! Detached sensory history: each slot records the last update plus the
//! previous origin as `last_visible_origin`. Linking, collision, and
//! authoritative actors remain shared; only observations live here.

use qa_core::math::Vec3;

/// Observation capacity (source entity slots).
pub const BOT_OBSERVATION_CAPACITY: usize = 1024;

/// Entity update pushed into the observation table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotEntityUpdate {
    /// Entity generation, when known.
    pub generation: Option<i32>,
    /// Entity type.
    pub entity_type: i32,
    /// Entity flags.
    pub flags: i32,
    /// Origin.
    pub origin: Vec3,
    /// Angles.
    pub angles: Vec3,
    /// Previous origin.
    pub old_origin: Vec3,
    /// Bounds mins.
    pub mins: Vec3,
    /// Bounds maxs.
    pub maxs: Vec3,
    /// Ground entity.
    pub ground_entity: i32,
    /// Solid.
    pub solid: i32,
    /// Model index.
    pub model_index: i32,
    /// Second model index.
    pub model_index2: i32,
    /// Frame.
    pub frame: i32,
    /// Event.
    pub event: i32,
    /// Event parameter.
    pub event_parameter: i32,
    /// Powerups.
    pub powerups: i32,
    /// Weapon.
    pub weapon: i32,
    /// Legs animation.
    pub legs_animation: i32,
    /// Torso animation.
    pub torso_animation: i32,
}

/// Stored entity info with validity and timing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasEntityInfo {
    /// Update payload.
    pub update: BotEntityUpdate,
    /// Whether the slot holds a live observation.
    pub valid: bool,
    /// Entity number.
    pub number: i32,
    /// Origin at the previous update.
    pub last_visible_origin: Vec3,
    /// Last update time.
    pub last_update_time: f32,
    /// Interval since the previous update.
    pub update_interval: f32,
}

fn zero() -> Vec3 {
    Vec3 { x: 0.0, y: 0.0, z: 0.0 }
}

fn empty_update() -> BotEntityUpdate {
    BotEntityUpdate {
        generation: None,
        entity_type: 0,
        flags: 0,
        origin: zero(),
        angles: zero(),
        old_origin: zero(),
        mins: zero(),
        maxs: zero(),
        ground_entity: 0,
        solid: 0,
        model_index: 0,
        model_index2: 0,
        frame: 0,
        event: 0,
        event_parameter: 0,
        powerups: 0,
        weapon: 0,
        legs_animation: 0,
        torso_animation: 0,
    }
}

fn empty_info(number: i32) -> AasEntityInfo {
    AasEntityInfo {
        update: empty_update(),
        valid: false,
        number,
        last_visible_origin: zero(),
        last_update_time: 0.0,
        update_interval: 0.0,
    }
}

/// Detached entity observation table.
#[derive(Debug, Clone)]
pub struct BotEntityObservations {
    records: Vec<AasEntityInfo>,
}

impl BotEntityObservations {
    /// Table with `capacity` slots.
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            records: (0..capacity).map(|number| empty_info(number as i32)).collect(),
        }
    }

    /// Slot capacity.
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.records.len()
    }

    /// Invalidate all slots.
    pub fn invalidate(&mut self) {
        for record in &mut self.records {
            record.valid = false;
        }
    }

    /// Push an update; a generation change resets the slot first.
    /// A `None` source leaves the slot untouched.
    pub fn update(&mut self, number: i32, source: Option<&BotEntityUpdate>, time: f32) {
        let Some(record) = self.records.get_mut(number as usize) else {
            return;
        };
        let Some(source) = source else {
            return;
        };
        if record.update.generation != source.generation {
            *record = empty_info(number);
        }
        let previous_time = record.last_update_time;
        let previous_origin = record.update.origin;
        record.update = *source;
        record.valid = true;
        record.last_visible_origin = previous_origin;
        record.last_update_time = time;
        record.update_interval = time - previous_time;
    }

    /// Read slot info; out-of-range slots read empty.
    #[must_use]
    pub fn info(&self, number: i32) -> AasEntityInfo {
        self.records
            .get(number as usize)
            .copied()
            .unwrap_or_else(|| empty_info(number))
    }

    /// Next valid slot after `after`, or 0.
    #[must_use]
    pub fn next_entity(&self, after: i32) -> i32 {
        let start = (after + 1).max(0) as usize;
        for (index, record) in self.records.iter().enumerate().skip(start) {
            if record.valid {
                return index as i32;
            }
        }
        0
    }

    /// Checkpoint all records.
    #[must_use]
    pub fn checkpoint(&self) -> Vec<AasEntityInfo> {
        self.records.clone()
    }

    /// Restore checkpointed records.
    pub fn restore(&mut self, records: &[AasEntityInfo]) {
        if records.len() == self.records.len() {
            self.records = records.to_vec();
        }
    }
}

impl Default for BotEntityObservations {
    fn default() -> Self {
        Self::new(BOT_OBSERVATION_CAPACITY)
    }
}
