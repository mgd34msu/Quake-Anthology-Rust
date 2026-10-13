//! Bounded configuration rows shared by SERVER modules and CLIENT connections.
use crate::primitives::{IndexSpan, RuleSetId};

pub const MAX_TEXT: usize = 8192;
pub const Q3_GAME_STATE_ROWS: usize = 1024;
pub const Q3_GAME_STATE_DATA_BYTES: usize = 16000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConfigError {
    Capacity,
    Index,
    Text,
}

struct Row {
    bytes: IndexSpan,
    length: usize,
    integer: i32,
    revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigRef<'a> {
    pub text: &'a [u8],
    pub integer: i32,
    pub revision: u64,
}

/// All row bytes and metadata are reserved at load. Mutation never grows them.
pub struct ConfigStrings {
    bytes: Box<[u8]>,
    rows: Box<[Row]>,
}
impl ConfigStrings {
    pub fn load(capacities: impl IntoIterator<Item = usize>) -> Result<Self, ConfigError> {
        let mut rows = Vec::new();
        let mut size = 0usize;
        for capacity in capacities {
            if rows.len() == 65536 || capacity > MAX_TEXT {
                return Err(ConfigError::Capacity);
            }
            let first = u32::try_from(size).map_err(|_| ConfigError::Capacity)?;
            size = size.checked_add(capacity).ok_or(ConfigError::Capacity)?;
            if size > 512 * 1024 * 1024 {
                return Err(ConfigError::Capacity);
            }
            rows.push(Row {
                bytes: IndexSpan {
                    first,
                    count: capacity as u32,
                },
                length: 0,
                integer: 0,
                revision: 0,
            });
        }
        Ok(Self {
            bytes: vec![0; size].into_boxed_slice(),
            rows: rows.into_boxed_slice(),
        })
    }
    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn capacity(&self, index: usize) -> Option<usize> {
        Some(self.rows.get(index)?.bytes.count as usize)
    }
    pub fn config(&self, index: usize) -> Option<ConfigRef<'_>> {
        let row = self.rows.get(index)?;
        let first = row.bytes.first as usize;
        Some(ConfigRef {
            text: &self.bytes[first..first + row.length],
            integer: row.integer,
            revision: row.revision,
        })
    }
    /// Rejection preserves the previous text, numeric projection and revision.
    pub fn set(&mut self, index: usize, text: &[u8]) -> Result<bool, ConfigError> {
        let row = self.rows.get_mut(index).ok_or(ConfigError::Index)?;
        if text.len() > row.bytes.count as usize || text.contains(&0) {
            return Err(ConfigError::Text);
        }
        let first = row.bytes.first as usize;
        if &self.bytes[first..first + row.length] == text {
            return Ok(false);
        }
        self.bytes[first..first + text.len()].copy_from_slice(text);
        row.length = text.len();
        row.integer = crate::text::atoi_prefix(text);
        row.revision = row.revision.wrapping_add(1);
        Ok(true)
    }
    pub fn reset(&mut self) {
        for row in &mut self.rows {
            if row.length != 0 {
                row.length = 0;
                row.integer = 0;
                row.revision = row.revision.wrapping_add(1);
            }
        }
    }
}

/// Module storage indices are source data, independent of movement and map.
/// Q1/QW rows are internal storage, not native wire configstring numbers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfigLayout {
    pub rules: RuleSetId,
    pub count: usize,
    /// Model, sound and image ordinals, in that order. An absent range is empty.
    pub resources: [IndexSpan; 3],
    pub players: IndexSpan,
    pub lights: IndexSpan,
    items: IndexSpan,
    general: IndexSpan,
    statusbar: IndexSpan,
    scalar: [usize; 3],
    ordinary_bytes: usize,
    light_bytes: usize,
    general_end: usize,
}
const fn span(first: u32, count: u32) -> IndexSpan {
    IndexSpan { first, count }
}
impl ConfigLayout {
    pub const Q3_SYSTEM_INFO: usize = 1;

    pub const QUAKE: Self = Self {
        rules: RuleSetId::Quake,
        count: 576,
        resources: [span(0, 256), span(256, 256), span(512, 0)],
        players: span(0, 0),
        lights: span(512, 64),
        items: span(0, 0),
        general: span(0, 0),
        statusbar: span(0, 0),
        scalar: [usize::MAX; 3],
        ordinary_bytes: MAX_TEXT + 1,
        light_bytes: 64,
        general_end: 576,
    };
    pub const QUAKEWORLD: Self = Self {
        rules: RuleSetId::QuakeWorld,
        ..Self::QUAKE
    };
    pub const QUAKE2: Self = Self {
        rules: RuleSetId::Quake2,
        count: 2080,
        resources: [span(32, 256), span(288, 256), span(544, 256)],
        players: span(1312, 256),
        lights: span(800, 256),
        items: span(1056, 256),
        general: span(1568, 512),
        statusbar: span(5, 24),
        scalar: [29, 30, 31],
        ordinary_bytes: 64,
        light_bytes: 64,
        general_end: 2080,
    };
    pub const QUAKE2_RERELEASE: Self = Self {
        rules: RuleSetId::Quake2Rerelease,
        count: 12448,
        resources: [span(62, 8192), span(8254, 2048), span(10302, 512)],
        players: span(11582, 256),
        lights: span(10814, 256),
        items: span(11326, 256),
        general: span(11838, 512),
        statusbar: span(5, 54),
        scalar: [59, 60, 61],
        ordinary_bytes: 96,
        light_bytes: 96,
        general_end: 12448,
    };
    pub const QUAKE3: Self = Self {
        rules: RuleSetId::Quake3,
        count: Q3_GAME_STATE_ROWS,
        resources: [span(32, 256), span(288, 256), span(0, 0)],
        players: span(544, 64),
        lights: span(0, 0),
        items: span(0, 0),
        general: span(0, 0),
        statusbar: span(0, 0),
        scalar: [usize::MAX; 3],
        ordinary_bytes: 8192,
        light_bytes: 8192,
        general_end: Q3_GAME_STATE_ROWS,
    };
    pub const fn for_rules(rules: RuleSetId) -> Self {
        match rules {
            RuleSetId::Quake => Self::QUAKE,
            RuleSetId::QuakeWorld => Self::QUAKEWORLD,
            RuleSetId::Quake2 => Self::QUAKE2,
            RuleSetId::Quake2Rerelease => Self::QUAKE2_RERELEASE,
            RuleSetId::Quake3 => Self::QUAKE3,
        }
    }
    pub fn style_index(self, ordinal: usize) -> Option<usize> {
        self.lights.get(ordinal)
    }
    pub fn statusbar(self) -> Option<usize> {
        self.statusbar.get(0)
    }
    /// Maximum text bytes, excluding the native trailing NUL.
    pub fn capacity(self, index: usize) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        let bytes = if self.lights.contains(index) {
            self.light_bytes
        } else if self.statusbar.contains(index) {
            (self.statusbar.range().end - index) * self.ordinary_bytes
        } else if self.general.contains(index) {
            (self.general_end - index) * self.ordinary_bytes
        } else {
            self.ordinary_bytes
        };
        Some((bytes - 1).min(MAX_TEXT))
    }
    /// Translate a source ordinal into this target namespace. Unsupported rows
    /// and resource ordinals outside the target's native range are dropped.
    pub fn project_index(self, source: Self, index: usize) -> Option<usize> {
        if index >= source.count {
            return None;
        }
        if self.rules == source.rules {
            return Some(index);
        }
        for (from, to) in source
            .resources
            .into_iter()
            .zip(self.resources)
            .chain(std::iter::once((source.players, self.players)))
            .chain(std::iter::once((source.lights, self.lights)))
        {
            if from.contains(index) {
                return to.get(index - from.first as usize);
            }
        }
        if !matches!(self.rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
            || !matches!(source.rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
        {
            return None;
        }
        if index < 5 {
            return Some(index);
        }
        if let Some(position) = source.scalar.iter().position(|&row| row == index) {
            return Some(self.scalar[position]);
        }
        for (from, to) in [
            (source.statusbar, self.statusbar),
            (source.items, self.items),
            (source.general, self.general),
        ] {
            if from.contains(index) {
                return to.get(index - from.first as usize);
            }
        }
        None
    }
}
