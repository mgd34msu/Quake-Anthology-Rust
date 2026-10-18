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
    text: crate::text::TextSlots,
    rows: Box<[Row]>,
}
impl ConfigStrings {
    pub fn load(capacities: impl IntoIterator<Item = usize>) -> Result<Self, ConfigError> {
        let text = crate::text::TextSlots::load(
            capacities,
            crate::text::SlotsLimits {
                rows: 65536,
                width: MAX_TEXT,
                bytes: 512 * 1024 * 1024,
            },
        )
        .map_err(|_| ConfigError::Capacity)?;
        let rows = (0..text.len())
            .map(|_| Row {
                integer: 0,
                revision: 0,
            })
            .collect();
        Ok(Self { text, rows })
    }

    pub fn len(&self) -> usize {
        self.rows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }
    pub fn capacity(&self, index: usize) -> Option<usize> {
        self.text.capacity(index)
    }
    pub fn config(&self, index: usize) -> Option<ConfigRef<'_>> {
        let row = self.rows.get(index)?;
        Some(ConfigRef {
            text: self.text.get(index)?,
            integer: row.integer,
            revision: row.revision,
        })
    }
    /// Rejection preserves the previous text, numeric projection and revision.
    pub fn set(&mut self, index: usize, text: &[u8]) -> Result<bool, ConfigError> {
        let row = self.rows.get_mut(index).ok_or(ConfigError::Index)?;
        if text.contains(&0) {
            return Err(ConfigError::Text);
        }
        if !self.text.set(index, text).map_err(|_| ConfigError::Text)? {
            return Ok(false);
        }
        row.integer = crate::text::atoi_prefix(text);
        row.revision = row.revision.wrapping_add(1);
        Ok(true)
    }
    pub fn reset(&mut self) {
        for (index, row) in self.rows.iter_mut().enumerate() {
            if self.text.get(index).is_some_and(|text| !text.is_empty()) {
                let _ = self.text.set_length(index, 0);
                row.integer = 0;
                row.revision = row.revision.wrapping_add(1);
            }
        }
    }
}

/// Native null/special ordinals are excluded from filename lookup and allocation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ResourceIndexPolicy {
    pub reserved: Option<u32>,
}
impl ResourceIndexPolicy {
    pub const DEFAULT: Self = Self { reserved: None };

    /// Native ordinals, independently of their containing configstring span.
    pub const fn permits(self, ordinal: u32) -> bool {
        ordinal != 0 && !matches!(self.reserved, Some(reserved) if reserved == ordinal)
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
    pub resource_policies: [ResourceIndexPolicy; 3],
    pub players: IndexSpan,
    pub lights: IndexSpan,
    /// Numeric or named soundtrack row consumed by the engine client.
    /// Q3 music remains a cgame request through the shared services.
    pub cd_track: Option<usize>,
    items: IndexSpan,
    general: IndexSpan,
    statusbar: IndexSpan,
    scalar: [usize; 3],
    ordinary_bytes: usize,
    player_bytes: usize,
    light_bytes: usize,
    general_end: usize,
}
const fn span(first: u32, count: u32) -> IndexSpan {
    IndexSpan { first, count }
}
impl ConfigLayout {
    pub const Q3_SERVER_INFO: usize = 0;
    pub const Q3_SYSTEM_INFO: usize = 1;

    pub const QUAKE: Self = Self {
        rules: RuleSetId::Quake,
        count: 592,
        resources: [span(0, 256), span(256, 256), span(512, 0)],
        resource_policies: [ResourceIndexPolicy::DEFAULT; 3],
        players: span(576, 16),
        lights: span(512, 64),
        cd_track: None,
        items: span(0, 0),
        general: span(0, 0),
        statusbar: span(0, 0),
        scalar: [usize::MAX; 3],
        ordinary_bytes: MAX_TEXT + 1,
        player_bytes: 32,
        light_bytes: 64,
        general_end: 592,
    };
    pub const QUAKEWORLD: Self = Self {
        rules: RuleSetId::QuakeWorld,
        count: 608,
        players: span(576, 32),
        player_bytes: 196,
        general_end: 608,
        ..Self::QUAKE
    };
    pub const QUAKE2: Self = Self {
        rules: RuleSetId::Quake2,
        count: 2080,
        resources: [span(32, 256), span(288, 256), span(544, 256)],
        resource_policies: [ResourceIndexPolicy::DEFAULT; 3],
        players: span(1312, 256),
        lights: span(800, 256),
        cd_track: Some(1),
        items: span(1056, 256),
        general: span(1568, 512),
        statusbar: span(5, 24),
        scalar: [29, 30, 31],
        ordinary_bytes: 64,
        player_bytes: 64,
        light_bytes: 64,
        general_end: 2080,
    };
    pub const QUAKE2_RERELEASE: Self = Self {
        rules: RuleSetId::Quake2Rerelease,
        count: 12448,
        resources: [span(62, 8192), span(8254, 2048), span(10302, 512)],
        resource_policies: [
            ResourceIndexPolicy {
                reserved: Some(255),
            },
            ResourceIndexPolicy::DEFAULT,
            ResourceIndexPolicy::DEFAULT,
        ],
        players: span(11582, 256),
        lights: span(10814, 256),
        cd_track: Some(1),
        items: span(11326, 256),
        general: span(11838, 512),
        statusbar: span(5, 54),
        scalar: [59, 60, 61],
        ordinary_bytes: 96,
        player_bytes: 96,
        light_bytes: 96,
        general_end: 12448,
    };
    pub const QUAKE3: Self = Self {
        rules: RuleSetId::Quake3,
        count: Q3_GAME_STATE_ROWS,
        resources: [span(32, 256), span(288, 256), span(0, 0)],
        resource_policies: [ResourceIndexPolicy::DEFAULT; 3],
        players: span(544, 64),
        lights: span(0, 0),
        cd_track: None,
        items: span(0, 0),
        general: span(0, 0),
        statusbar: span(0, 0),
        scalar: [usize::MAX; 3],
        ordinary_bytes: 8192,
        player_bytes: 8192,
        light_bytes: 8192,
        general_end: Q3_GAME_STATE_ROWS,
    };
    /// Negotiated Q2rePRO game-3 namespaces retain the rerelease wire role.
    pub const fn quake2_compat(extended: bool) -> Self {
        let native = Self::QUAKE2_RERELEASE;
        // Protocol 1038 retains 96-byte cells with either game-3 namespace.
        let legacy = Self {
            rules: native.rules,
            ordinary_bytes: native.ordinary_bytes,
            player_bytes: native.player_bytes,
            light_bytes: native.light_bytes,
            ..Self::QUAKE2
        };
        if !extended {
            return legacy;
        }
        let images = span(native.resources[2].first, 2048);
        let lights = span(images.first + images.count, native.lights.count);
        let items = span(lights.first + lights.count, native.items.count);
        let players = span(items.first + items.count, native.players.count);
        let general = span(players.first + players.count, native.general.count);
        let end = (general.first + general.count) as usize;
        Self {
            rules: RuleSetId::Quake2Rerelease,
            count: end,
            resources: [native.resources[0], native.resources[1], images],
            resource_policies: native.resource_policies,
            players,
            lights,
            items,
            general,
            statusbar: native.statusbar,
            scalar: native.scalar,
            general_end: end,
            ..legacy
        }
    }
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
    /// Native Q2/RR maximum-client count; other layouts have no scalar row.
    pub fn max_clients(self) -> Option<usize> {
        self.scalar_row(1)
    }
    pub fn map_checksum(self) -> Option<usize> {
        self.scalar_row(2)
    }
    fn scalar_row(self, index: usize) -> Option<usize> {
        let row = self.scalar[index];
        (row != usize::MAX).then_some(row)
    }
    /// Maximum text bytes, excluding the native trailing NUL.
    pub fn capacity(self, index: usize) -> Option<usize> {
        if index >= self.count {
            return None;
        }
        let bytes = if self.players.contains(index) {
            self.player_bytes
        } else if self.lights.contains(index) {
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
        if self == source {
            return Some(index);
        }
        if source.players.contains(index) {
            return (matches!(self.rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease)
                && matches!(source.rules, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease))
            .then(|| self.players.get(index - source.players.first as usize))
            .flatten();
        }
        for (from, to) in source
            .resources
            .into_iter()
            .zip(self.resources)
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
