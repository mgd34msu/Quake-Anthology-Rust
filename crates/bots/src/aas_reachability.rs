//! Reachability construction from id Software `be_aas_reach.c`, ported
//! from `src/bots/navigation/aas-reachability.ts`. Geometry and movement
//! borrow the shared map and the selected prediction owner.
//! Copyright (C) 1999-2005 Id Software, Inc.
//!
//! The donor threads one shared-mutable context through its geometry,
//! special, and spatial classes. This port keeps that structure: the
//! [`AasReachabilityContext`] below owns all construction state, and the
//! `aas_reachability_geometry`, `aas_reachability_spatial`, and
//! `aas_reachability_special` modules extend it with inherent method
//! blocks. Link chains live in a slab instead of an intrusive list; the
//! float32/int32/uint16 field semantics of the donor's link records are
//! preserved by [`AasLinkedReachability`].

use std::collections::HashMap;

use qa_core::math::Vec3;

use crate::aas::{AasAreaSettings, AasAsset, AasReachability};
use crate::aas_reachability_spatial::AasReachabilityEntities;
use crate::aas_reachability_types::{
    init_aas_movement_settings, AasLibVarValue, AasMovementSettings, AasReachabilityWorld,
};
use crate::behavior::{BotMovementPrediction, BotTravelPredictionResult};
use crate::error::{indexed, BotsError};
use crate::scene::{DecodedWorld, SceneQueries};
use crate::types::NavigationProfile;

/// Source link record size in bytes (diagnostic shape only).
pub const LINK_BYTES: usize = 48;
/// Maximum allocated reachability links.
pub const MAX_REACHABILITY: usize = 65536;

/// Diagnostic printer by severity.
pub type AasSeverityPrinter = dyn Fn(u8, &str);

/// Reachability construction options.
pub struct AasReachabilityOptions<'a> {
    /// Source asset; returned unchanged when it already carries
    /// reachability unless `force` rebuilds.
    pub asset: &'a AasAsset,
    /// Decoded map geometry (entity text and model bounds).
    pub geometry: &'a DecodedWorld,
    /// Shared collision queries.
    pub scene: &'a dyn SceneQueries,
    /// Traversal profile selecting presence bounds.
    pub profile: &'a NavigationProfile,
    /// Client slot the predictor simulates as.
    pub prediction_client: i32,
    /// Client-movement predictor owned by the selected movement provider.
    pub predict_client_movement: &'a dyn Fn(BotMovementPrediction) -> BotTravelPredictionResult,
    /// Rebuild even when reachability exists.
    pub force: bool,
    /// Emit construction diagnostics.
    pub debug: bool,
    /// Movement settings override; otherwise initialized from variables.
    pub settings: Option<&'a AasMovementSettings>,
    /// Library-variable owner.
    pub variable: Option<&'a dyn AasLibVarValue>,
    /// Diagnostic printer by severity.
    pub print: Option<&'a AasSeverityPrinter>,
    /// Persistent debug-line sink.
    pub debug_line: Option<&'a dyn Fn(Vec3, Vec3, i32)>,
}

/// Per-category construction counters.
#[derive(Debug, Clone, Default)]
pub struct AasReachabilityDebugState {
    /// Counts by category.
    pub counts: HashMap<String, u64>,
}

impl AasReachabilityDebugState {
    /// Increment a category counter.
    pub fn count(&mut self, category: &str) {
        *self.counts.entry(category.to_string()).or_insert(0) += 1;
    }
}

/// Binary32 source link fields with ordinary Rust lifetime. `Vec3` stores
/// binary32 components, matching the donor's `DataView` float32 vector
/// fields; integer fields match its int32/uint16 storage.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AasLinkedReachability {
    /// Destination area.
    pub area: i32,
    /// Source face.
    pub face: i32,
    /// Source edge.
    pub edge: i32,
    /// Traversal start.
    pub start: Vec3,
    /// Traversal end.
    pub end: Vec3,
    /// Travel type with team bits.
    pub travel_type: i32,
    /// Travel time in centiseconds.
    pub travel_time: u16,
}

impl AasLinkedReachability {
    /// Zeroed link.
    #[must_use]
    pub fn zero() -> Self {
        Self {
            area: 0,
            face: 0,
            edge: 0,
            start: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            end: Vec3 { x: 0.0, y: 0.0, z: 0.0 },
            travel_type: 0,
            travel_time: 0,
        }
    }

    /// Store a travel time through the donor's conversion: truncate,
    /// range-check against signed int32, then store the low 16 bits.
    pub fn set_travel_time(&mut self, value: f64) -> Result<(), BotsError> {
        let integer = value.trunc();
        if !integer.is_finite() || integer < -2_147_483_648.0 || integer > 2_147_483_647.0 {
            return Err(BotsError::IntRange(
                "AAS reachability travel time exceeds source integer conversion range".to_string(),
            ));
        }
        self.travel_time = (integer as i32) as u16;
        Ok(())
    }

    /// Clear all fields.
    pub fn clear(&mut self) {
        *self = Self::zero();
    }
}

/// Slab slot: a link plus its chain successor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ReachSlot {
    /// Link payload.
    pub link: AasLinkedReachability,
    /// Next slot in the area chain.
    pub next: Option<usize>,
}

/// Shared construction state. Single-threaded; methods take `&mut self`
/// where the donor mutates the context.
pub(crate) struct AasReachabilityContext<'a> {
    /// Options borrowed from the caller.
    pub options: &'a AasReachabilityOptions<'a>,
    /// World under construction.
    pub world: AasReachabilityWorld,
    /// BSP entity records.
    pub bsp_entities: AasReachabilityEntities,
    /// Movement settings.
    pub movement_settings: AasMovementSettings,
    /// Construction diagnostics enabled.
    pub debug: bool,
    /// Category counters.
    pub debug_state: AasReachabilityDebugState,
    /// Per-area chain heads.
    pub heads: Vec<Option<usize>>,
    /// Link slab.
    pub slab: Vec<ReachSlot>,
    /// Recycled slot ids.
    pub free: Vec<usize>,
    /// Live allocation count.
    pub allocated: usize,
}

impl<'a> AasReachabilityContext<'a> {
    /// Print a diagnostic at a severity.
    pub fn print(&self, severity: u8, text: &str) {
        if let Some(print) = self.options.print {
            print(severity, text);
        }
    }

    /// Log an informational diagnostic.
    pub fn log(&self, text: &str) {
        self.print(1, text);
    }

    /// Emit a persistent debug line.
    pub fn permanent_line(&self, start: Vec3, end: Vec3, color: i32) {
        if let Some(line) = self.options.debug_line {
            line(start, end, color);
        }
    }

    /// Read a library variable with its source default text.
    pub fn variable(&self, name: &str, default: &str) -> f64 {
        match self.options.variable {
            Some(owner) => owner.value(name, default),
            None => default_number(default),
        }
    }

    /// Borrow a slot by id.
    pub fn slot(&self, id: usize) -> Result<&ReachSlot, BotsError> {
        indexed(&self.slab, id as i64, "AAS reachability index")
    }

    /// Mutably borrow a slot by id.
    pub fn slot_mut(&mut self, id: usize) -> Result<&mut ReachSlot, BotsError> {
        crate::error::indexed_mut(&mut self.slab, id as i64, "AAS reachability index")
    }

    /// Allocate a link, recycling a freed slot when one is available.
    /// Returns `None` at the source cap after printing the overflow.
    pub fn allocate(&mut self) -> Option<usize> {
        if self.allocated >= MAX_REACHABILITY {
            self.print(4, "AAS_MAX_REACHABILITYSIZE");
            return None;
        }
        self.allocated += 1;
        if let Some(id) = self.free.pop() {
            return Some(id);
        }
        let id = self.slab.len();
        self.slab.push(ReachSlot {
            link: AasLinkedReachability::zero(),
            next: None,
        });
        Some(id)
    }

    /// Recycle a link.
    pub fn free(&mut self, id: usize) -> Result<(), BotsError> {
        let slot = self.slot_mut(id)?;
        slot.link.clear();
        slot.next = None;
        self.free.push(id);
        self.allocated = self.allocated.saturating_sub(1);
        Ok(())
    }

    /// Prepend a link to an area chain.
    pub fn link_area(&mut self, area: i32, id: usize) -> Result<(), BotsError> {
        let head = *indexed(&self.heads, i64::from(area), "AAS reachability index")?;
        self.slot_mut(id)?.next = head;
        let length = self.heads.len();
        let slot = self.heads.get_mut(area as usize).ok_or(BotsError::OutOfRange {
            what: "AAS reachability index",
            index: i64::from(area),
            length,
        })?;
        *slot = Some(id);
        Ok(())
    }

    /// Whether an area already links to another area.
    pub fn exists(&self, from: i32, to: i32) -> Result<bool, BotsError> {
        let mut next = *indexed(&self.heads, i64::from(from), "AAS reachability index")?;
        while let Some(id) = next {
            let slot = self.slot(id)?;
            if slot.link.area == to {
                return Ok(true);
            }
            next = slot.next;
        }
        Ok(false)
    }
}

fn default_number(text: &str) -> f64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0.0;
    }
    trimmed.parse().unwrap_or(f64::NAN)
}

/// Rebuild source links without mutating the input asset or its live
/// movement actor.
pub fn build_aas_reachability(options: &AasReachabilityOptions<'_>) -> Result<AasAsset, BotsError> {
    if !options.asset.reachability.is_empty() && !options.force {
        return Ok(options.asset.clone());
    }
    if options.prediction_client < 0 {
        return Err(BotsError::BadPredictionClient);
    }
    let settings: Vec<AasAreaSettings> = options.asset.settings.clone();
    let world = AasReachabilityWorld {
        asset: options.asset.clone(),
        settings,
    };
    let movement_settings = match options.settings {
        Some(settings) => settings.clone(),
        None => {
            let mut settings = AasMovementSettings::default();
            struct Defaults<'x> {
                options: &'x AasReachabilityOptions<'x>,
            }
            impl AasLibVarValue for Defaults<'_> {
                fn value(&self, name: &str, default: &str) -> f64 {
                    match self.options.variable {
                        Some(owner) => owner.value(name, default),
                        None => default_number(default),
                    }
                }
            }
            let defaults = Defaults { options };
            init_aas_movement_settings(&defaults, &mut settings);
            settings
        }
    };
    let bsp_entities = AasReachabilityEntities::new(options.geometry.entities())?;
    let heads = vec![None; world.asset.areas.len()];
    let mut context = AasReachabilityContext {
        options,
        world,
        bsp_entities,
        movement_settings,
        debug: options.debug,
        debug_state: AasReachabilityDebugState::default(),
        heads,
        slab: Vec::new(),
        free: Vec::new(),
        allocated: 0,
    };
    context.set_weapon_jump_area_flags()?;
    let grapple = context.variable("grapplereach", "0").trunc() != 0.0;
    let area_count = context.world.asset.areas.len() as i32;
    for from in 1..area_count {
        if (context.world.setting(from)?.contents & 128) != 0 {
            continue;
        }
        for to in 1..area_count {
            if from == to || context.exists(from, to)? {
                continue;
            }
            if (context.world.setting(from)?.contents & (64 | 128)) != 0
                && (context.world.setting(to)?.contents & (64 | 128)) == 0
            {
                continue;
            }
            if context.swim(from, to)?
                || context.equal_floor_height(from, to)?
                || context.step_barrier_water_jump_walk_off_ledge(from, to)?
                || context.ladder(from, to)?
                || context.jump(from, to)?
            {
                continue;
            }
        }
        if (context.world.setting(from)?.contents & (64 | 128)) != 0 {
            continue;
        }
        for to in 1..area_count {
            if from == to || context.exists(from, to)? {
                continue;
            }
            if grapple {
                context.grapple(from, to)?;
            }
            context.weapon_jump(from, to)?;
        }
    }
    for area in 1..area_count {
        if (context.world.setting(area)?.contents & 128) == 0 {
            context.walk_off_ledge(area)?;
        }
    }
    context.jump_pad()?;
    context.teleport()?;
    context.elevator()?;
    context.func_bobbing()?;
    let zero = Vec3 { x: 0.0, y: 0.0, z: 0.0 };
    let mut reachability = vec![AasReachability {
        area: 0,
        face: 0,
        edge: 0,
        start: zero,
        end: zero,
        travel_type: 0,
        travel_time: 0,
        padding: 0,
    }];
    for area in 0..area_count {
        {
            let setting = context.world.setting_mut(area)?;
            setting.first_reach = reachability.len() as i32;
            setting.reach_count = 0;
        }
        let mut next = *indexed(&context.heads, i64::from(area), "AAS reachability index")?;
        while let Some(id) = next {
            let slot = context.slot(id)?;
            let link = slot.link;
            next = slot.next;
            reachability.push(AasReachability {
                area: link.area,
                face: link.face,
                edge: link.edge,
                start: link.start,
                end: link.end,
                travel_type: link.travel_type,
                travel_time: link.travel_time,
                padding: 0,
            });
            context.world.setting_mut(area)?.reach_count += 1;
        }
    }
    Ok(AasAsset {
        settings: context.world.settings.clone(),
        reachability,
        ..options.asset.clone()
    })
}
