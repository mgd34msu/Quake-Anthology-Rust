//! Goal and item library from `src/bots/behavior/library/goals.ts`
//! (`be_ai_goal.c`: `BotLoadItemWeights`, `BotChooseLTGItem`,
//! `BotChooseNBGItem`, `BotPushGoal`/`BotPopGoal`, avoid goals).
//!
//! Item configs declare pickups (classname, model, bounds, respawn).
//! The goal library tracks level items, per-bot goal stacks, and timed
//! avoid goals, and picks long-term (`BotChooseLTGItem`) and nearby
//! (`BotChooseNBGItem`) goals by fuzzy item weight over travel time.

use std::collections::HashMap;

use qa_core::math::Vec3;

use crate::behavior::assets::BotSourceFiles;
use crate::behavior::library::structure::read_structure_definitions;
use crate::error::BotsError;

/// Goal flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GoalFlags;

impl GoalFlags {
    /// Item goal.
    pub const ITEM: i32 = 1;
    /// Roam goal.
    pub const ROAM: i32 = 2;
    /// Dropped-item goal.
    pub const DROPPED: i32 = 4;
}

/// Goal library load errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct GoalError;

impl GoalError {
    /// No error.
    pub const NONE: i32 = 0;
    /// Item weights failed to load.
    pub const CANNOT_LOAD_ITEM_WEIGHTS: i32 = 9;
    /// Item config failed to load.
    pub const CANNOT_LOAD_ITEM_CONFIG: i32 = 10;
}

/// Maximum goal states.
pub const MAX_GOAL_STATES: usize = 64;
/// Maximum goal stack depth.
pub const MAX_GOAL_STACK: usize = 8;
/// Maximum avoid goals.
pub const MAX_AVOID_GOALS: usize = 256;
/// Source-owned goal numbers start here.
pub const SOURCE_GOAL_NUMBER_MIN: i32 = 0x4000_0000;

/// Whether a goal number is source-owned.
#[must_use]
pub fn is_source_goal_number(number: i32) -> bool {
    number >= SOURCE_GOAL_NUMBER_MIN
}

/// One bot goal (`bot_goal_t`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BotGoal {
    /// Goal origin.
    pub origin: Vec3,
    /// Goal area.
    pub area: i32,
    /// Goal bounds mins.
    pub mins: Vec3,
    /// Goal bounds maxs.
    pub maxs: Vec3,
    /// Goal entity, or -1.
    pub entity: i32,
    /// Goal number.
    pub number: i32,
    /// Goal flags.
    pub flags: i32,
    /// Item info index.
    pub item_info: i32,
}

impl Default for BotGoal {
    fn default() -> Self {
        Self {
            origin: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            area: 0,
            mins: Vec3 {
                x: -15.0,
                y: -15.0,
                z: -15.0,
            },
            maxs: Vec3 {
                x: 15.0,
                y: 15.0,
                z: 15.0,
            },
            entity: -1,
            number: 0,
            flags: 0,
            item_info: 0,
        }
    }
}

impl BotGoal {
    /// Copy fields from another goal.
    pub fn copy_from(&mut self, other: &BotGoal) {
        *self = *other;
    }
}

/// Whether an origin touches a goal (`BotTouchingGoal`): inside the
/// goal bounds expanded by the item interaction radius.
#[must_use]
pub fn touching_goal(origin: Vec3, goal: &BotGoal) -> bool {
    const TOUCH: f32 = 10.0;
    origin.x >= goal.origin.x + goal.mins.x - TOUCH
        && origin.x <= goal.origin.x + goal.maxs.x + TOUCH
        && origin.y >= goal.origin.y + goal.mins.y - TOUCH
        && origin.y <= goal.origin.y + goal.maxs.y + TOUCH
        && origin.z >= goal.origin.z + goal.mins.z - TOUCH
        && origin.z <= goal.origin.z + goal.maxs.z + TOUCH
}

/// Item definition (`iteminfo_t`).
#[derive(Debug, Clone, PartialEq)]
pub struct ItemInfo {
    /// Classname.
    pub classname: String,
    /// Display name.
    pub name: String,
    /// Model name.
    pub model: String,
    /// Model index.
    pub model_index: i32,
    /// Item type.
    pub item_type: i32,
    /// Item index.
    pub index: i32,
    /// Respawn time seconds.
    pub respawn_time: f32,
    /// Bounds mins.
    pub mins: Vec3,
    /// Bounds maxs.
    pub maxs: Vec3,
    /// Item number.
    pub number: i32,
}

/// Parsed item config.
#[derive(Debug, Clone, Default)]
pub struct ItemConfig {
    /// Source path.
    pub path: String,
    /// Declared items.
    pub items: Vec<ItemInfo>,
    /// Parse warnings.
    pub diagnostics: Vec<String>,
}

impl ItemConfig {
    /// Parse `iteminfo` blocks.
    pub fn parse(path: &str, text: &str) -> Result<Self, BotsError> {
        let definitions = read_structure_definitions(text)?;
        let mut items = Vec::new();
        for definition in &definitions {
            if definition.type_name.as_deref() != Some("iteminfo") {
                continue;
            }
            let number = definition.number("number").unwrap_or(items.len() as f64) as i32;
            items.push(ItemInfo {
                classname: definition.string("classname").unwrap_or_default().to_owned(),
                name: definition.string("name").unwrap_or_default().to_owned(),
                model: definition.string("model").unwrap_or_default().to_owned(),
                model_index: definition.number("modelindex").unwrap_or(0.0) as i32,
                item_type: definition.number("type").unwrap_or(0.0) as i32,
                index: definition.number("index").unwrap_or(0.0) as i32,
                respawn_time: definition.number("respawntime").unwrap_or(0.0) as f32,
                mins: Vec3 {
                    x: definition.number("mins.x").unwrap_or(-15.0) as f32,
                    y: definition.number("mins.y").unwrap_or(-15.0) as f32,
                    z: definition.number("mins.z").unwrap_or(-15.0) as f32,
                },
                maxs: Vec3 {
                    x: definition.number("maxs.x").unwrap_or(15.0) as f32,
                    y: definition.number("maxs.y").unwrap_or(15.0) as f32,
                    z: definition.number("maxs.z").unwrap_or(15.0) as f32,
                },
                number,
            });
        }
        Ok(Self {
            path: path.to_owned(),
            items,
            diagnostics: Vec::new(),
        })
    }

    /// Item by classname.
    #[must_use]
    pub fn by_classname(&self, classname: &str) -> Option<&ItemInfo> {
        self.items.iter().find(|item| item.classname == classname)
    }
}

/// Level item instance tracked by the goal library.
#[derive(Debug, Clone, PartialEq)]
pub struct LevelItem {
    /// Goal number.
    pub number: i32,
    /// Item info index.
    pub item_info: usize,
    /// Entity number, or -1.
    pub entity: i32,
    /// Origin.
    pub origin: Vec3,
    /// Area number.
    pub area: i32,
    /// Goal flags.
    pub flags: i32,
    /// Weight override (fuzzy weight when zero means evaluate).
    pub weight: f32,
    /// Time the item becomes available again.
    pub timeout: f32,
}

/// Timed avoid goal.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AvoidGoal {
    /// Goal number.
    pub number: i32,
    /// Time the avoidance expires.
    pub expire_time: f32,
}

/// Per-bot goal state: stack plus avoid list.
#[derive(Debug, Clone, Default)]
pub struct BotGoalState {
    /// Goal stack, top at the end.
    pub stack: Vec<BotGoal>,
    /// Avoid goals.
    pub avoid: Vec<AvoidGoal>,
    /// Item weight overrides by goal number.
    pub weights: HashMap<i32, f32>,
}

/// Goal library: items plus per-bot goal states.
pub struct BotGoalLibrary<'a> {
    files: &'a dyn BotSourceFiles,
    /// Loaded item config.
    pub item_config: Option<ItemConfig>,
    /// Level items.
    pub level_items: Vec<LevelItem>,
    states: Vec<Option<BotGoalState>>,
    next_number: i32,
}

impl<'a> BotGoalLibrary<'a> {
    /// New goal library over prepared files.
    pub fn new(files: &'a dyn BotSourceFiles) -> Self {
        Self {
            files,
            item_config: None,
            level_items: Vec::new(),
            states: vec![None; MAX_GOAL_STATES],
            next_number: 1,
        }
    }

    /// Load the item config (`BotLoadItemWeights` config half).
    pub fn load_item_config(&mut self, path: &str) -> i32 {
        let bytes = match self.files.read(path) {
            Some(bytes) => bytes,
            None => return GoalError::CANNOT_LOAD_ITEM_CONFIG,
        };
        let text = String::from_utf8_lossy(&bytes);
        match ItemConfig::parse(path, &text) {
            Ok(config) => {
                self.item_config = Some(config);
                GoalError::NONE
            }
            Err(_) => GoalError::CANNOT_LOAD_ITEM_CONFIG,
        }
    }

    /// Allocate a goal state (`BotAllocGoalState`).
    pub fn alloc_goal_state(&mut self) -> Option<i32> {
        self.states.iter().position(Option::is_none).map(|index| {
            self.states[index] = Some(BotGoalState::default());
            index as i32 + 1
        })
    }

    /// Free a goal state (`BotFreeGoalState`).
    pub fn free_goal_state(&mut self, handle: i32) {
        if let Some(slot) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            *slot = None;
        }
    }

    /// Reset a goal state (`BotResetGoalState`).
    pub fn reset_goal_state(&mut self, handle: i32) {
        if let Some(Some(state)) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            state.stack.clear();
            state.avoid.clear();
            state.weights.clear();
        }
    }

    /// Reset avoid goals (`BotResetAvoidGoals`).
    pub fn reset_avoid_goals(&mut self, handle: i32) {
        if let Some(Some(state)) = handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize))
        {
            state.avoid.clear();
        }
    }

    fn state(&self, handle: i32) -> Option<&BotGoalState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get(index as usize)?.as_ref())
    }

    fn state_mut(&mut self, handle: i32) -> Option<&mut BotGoalState> {
        handle
            .checked_sub(1)
            .and_then(|index| self.states.get_mut(index as usize)?.as_mut())
    }

    /// Push a goal (`BotPushGoal`); drops the bottom past `MAX_GOAL_STACK`.
    pub fn push_goal(&mut self, handle: i32, goal: BotGoal) {
        if let Some(state) = self.state_mut(handle) {
            if state.stack.len() >= MAX_GOAL_STACK {
                state.stack.remove(0);
            }
            state.stack.push(goal);
        }
    }

    /// Pop a goal (`BotPopGoal`).
    pub fn pop_goal(&mut self, handle: i32) {
        if let Some(state) = self.state_mut(handle) {
            state.stack.pop();
        }
    }

    /// Top goal (`BotGetTopGoal`).
    #[must_use]
    pub fn top_goal(&self, handle: i32) -> Option<BotGoal> {
        self.state(handle)?.stack.last().copied()
    }

    /// Second goal (`BotGetSecondGoal`).
    #[must_use]
    pub fn second_goal(&self, handle: i32) -> Option<BotGoal> {
        let stack = &self.state(handle)?.stack;
        stack.get(stack.len().checked_sub(2)?).copied()
    }

    /// Empty the stack (`BotEmptyGoalStack`).
    pub fn empty_goal_stack(&mut self, handle: i32) {
        if let Some(state) = self.state_mut(handle) {
            state.stack.clear();
        }
    }

    /// Add an avoid goal (`BotAddAvoidGoal`).
    pub fn add_avoid_goal(&mut self, handle: i32, number: i32, expire_time: f32) {
        if let Some(state) = self.state_mut(handle) {
            if let Some(existing) = state.avoid.iter_mut().find(|avoid| avoid.number == number) {
                existing.expire_time = expire_time;
                return;
            }
            if state.avoid.len() >= MAX_AVOID_GOALS {
                state.avoid.remove(0);
            }
            state.avoid.push(AvoidGoal { number, expire_time });
        }
    }

    /// Remove an avoid goal (`BotRemoveFromAvoidGoals`).
    pub fn remove_avoid_goal(&mut self, handle: i32, number: i32) {
        if let Some(state) = self.state_mut(handle) {
            state.avoid.retain(|avoid| avoid.number != number);
        }
    }

    /// Whether a goal is avoided at `time`.
    #[must_use]
    pub fn is_avoided(&self, handle: i32, number: i32, time: f32) -> bool {
        self.state(handle).is_some_and(|state| {
            state
                .avoid
                .iter()
                .any(|avoid| avoid.number == number && avoid.expire_time > time)
        })
    }

    /// Add a level item; returns its goal number.
    pub fn add_level_item(&mut self, item_info: usize, entity: i32, origin: Vec3, area: i32, flags: i32) -> i32 {
        let number = self.next_number;
        self.next_number += 1;
        self.level_items.push(LevelItem {
            number,
            item_info,
            entity,
            origin,
            area,
            flags,
            weight: 0.0,
            timeout: 0.0,
        });
        number
    }

    /// Remove level items for an entity.
    pub fn remove_entity_items(&mut self, entity: i32) {
        self.level_items.retain(|item| item.entity != entity);
    }

    /// Clear level items.
    pub fn clear_level_items(&mut self) {
        self.level_items.clear();
        self.next_number = 1;
    }

    /// Choose the best long-term item goal (`BotChooseLTGItem`).
    ///
    /// Scores available, un-avoided items by fuzzy weight over travel
    /// time as reported by the caller's `travel_time` closure.
    pub fn choose_ltg_item(
        &self,
        handle: i32,
        origin: Vec3,
        weights: &dyn Fn(&LevelItem) -> f32,
        travel_time: &dyn Fn(i32, i32) -> i32,
        origin_area: i32,
        time: f32,
    ) -> Option<BotGoal> {
        let _ = origin;
        let mut best: Option<(f32, &LevelItem)> = None;
        for item in &self.level_items {
            if item.timeout > time || self.is_avoided(handle, item.number, time) {
                continue;
            }
            let weight = weights(item);
            if weight <= 0.0 {
                continue;
            }
            let travel = travel_time(origin_area, item.area).max(1) as f32;
            let score = weight / travel;
            if best.is_none_or(|(best_score, _)| score > best_score) {
                best = Some((score, item));
            }
        }
        best.map(|(_, item)| self.goal_for(item))
    }

    /// Choose the best nearby item goal (`BotChooseNBGItem`).
    pub fn choose_nbg_item(
        &self,
        handle: i32,
        origin: Vec3,
        radius: f32,
        weights: &dyn Fn(&LevelItem) -> f32,
        time: f32,
    ) -> Option<BotGoal> {
        let mut best: Option<(f32, &LevelItem)> = None;
        for item in &self.level_items {
            if item.timeout > time || self.is_avoided(handle, item.number, time) {
                continue;
            }
            let dx = item.origin.x - origin.x;
            let dy = item.origin.y - origin.y;
            let dz = item.origin.z - origin.z;
            let distance = (dx * dx + dy * dy + dz * dz).sqrt();
            if distance > radius {
                continue;
            }
            let weight = weights(item);
            if weight <= 0.0 {
                continue;
            }
            let score = weight / distance.max(1.0);
            if best.is_none_or(|(best_score, _)| score > best_score) {
                best = Some((score, item));
            }
        }
        best.map(|(_, item)| self.goal_for(item))
    }

    /// Build a `BotGoal` for a level item.
    #[must_use]
    pub fn goal_for(&self, item: &LevelItem) -> BotGoal {
        let (mins, maxs, info) = self
            .item_config
            .as_ref()
            .and_then(|config| config.items.get(item.item_info))
            .map(|info| (info.mins, info.maxs, info.number))
            .unwrap_or((
                Vec3 {
                    x: -15.0,
                    y: -15.0,
                    z: -15.0,
                },
                Vec3 {
                    x: 15.0,
                    y: 15.0,
                    z: 15.0,
                },
                0,
            ));
        BotGoal {
            origin: item.origin,
            area: item.area,
            mins,
            maxs,
            entity: item.entity,
            number: item.number,
            flags: item.flags | GoalFlags::ITEM,
            item_info: info,
        }
    }

    /// Goal stack depth.
    #[must_use]
    pub fn stack_depth(&self, handle: i32) -> usize {
        self.state(handle).map_or(0, |state| state.stack.len())
    }
}
