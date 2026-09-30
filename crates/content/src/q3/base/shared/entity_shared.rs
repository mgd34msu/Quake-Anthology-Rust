//! Quake III base/shared: entity shared.
//!
//! Donor provenance: `src/content/q3/base/shared/entity-shared.ts`.

use qa_core::math::{vec3, Vec3};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::mirrors::*;
use crate::q3::base::shared::entity_state::*;

// ---------------------------------------------------------------------------
// shared/entity-shared.ts
// ---------------------------------------------------------------------------

/// Entity collision model (`EntityCollisionModel`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityCollisionModel {
    /// Inline BSP model with its index.
    Inline {
        /// Model index.
        index: i32,
    },
    /// Bounding box.
    Box,
    /// Capsule.
    Capsule,
}

/// Server entity flags (`ServerEntityFlags`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(i32)]
pub enum ServerEntityFlags {
    /// Not networked to clients.
    Noclient = 1,
    /// Client mask follows.
    Clientmask = 2,
    /// Bot.
    Bot = 8,
    /// Broadcast.
    Broadcast = 32,
    /// Portal.
    Portal = 64,
    /// Use current origin.
    UseCurrentOrigin = 128,
    /// Single client follows.
    Singleclient = 256,
    /// Excluded from server info.
    Noserverinfo = 512,
    /// Inverted single client.
    Notsingleclient = 2048,
}

impl ServerEntityFlags {
    /// Flag bits.
    #[must_use]
    pub fn bits(self) -> i32 {
        self as i32
    }
}

/// Body fields forwarded to the shared authority (`EntityBodyBinding`).
pub trait EntityBodyBinding {
    /// Read the body state.
    fn read(&self) -> BodyState;
    /// Write the body state.
    fn write(&self, value: BodyState);
    /// Read the linked body, if linked.
    fn linked(&self) -> Option<LinkedBody>;
}

/// Private entity-shared save words.
#[derive(Debug, Clone, PartialEq)]
pub struct PrivateEntityState {
    /// Previous link record.
    pub previous_link: Option<LinkedBody>,
    /// Absolute minimum override.
    pub abs_min_override: Option<Vec3>,
    /// Absolute maximum override.
    pub abs_max_override: Option<Vec3>,
}

/// Source `entityShared_t` metadata (`EntityShared`).
///
/// Body fields forward to the shared authority; the remaining words are
/// source metadata.
pub struct EntityShared {
    body: Rc<dyn EntityBodyBinding>,
    previous_link: Option<LinkedBody>,
    current_origin_view: Option<Vec3>,
    abs_min_override: Option<Vec3>,
    abs_max_override: Option<Vec3>,
    /// Server flags.
    pub sv_flags: i32,
    /// Single-client target.
    pub single_client: i32,
    /// Collision model.
    pub model: EntityCollisionModel,
    /// Contents mask.
    pub contents: i32,
    /// Owner entity number.
    pub owner_num: i32,
}

impl std::fmt::Debug for EntityShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EntityShared")
            .field("sv_flags", &self.sv_flags)
            .field("model", &self.model)
            .field("contents", &self.contents)
            .field("owner_num", &self.owner_num)
            .finish()
    }
}

impl EntityShared {
    /// Shared metadata over a body binding.
    #[must_use]
    pub fn new(body: Rc<dyn EntityBodyBinding>) -> Self {
        Self {
            body,
            previous_link: None,
            current_origin_view: None,
            abs_min_override: None,
            abs_max_override: None,
            sv_flags: 0,
            single_client: 0,
            model: EntityCollisionModel::Box,
            contents: 0,
            owner_num: 0,
        }
    }

    /// Whether the body is linked.
    #[must_use]
    pub fn linked(&self) -> bool {
        self.body.linked().is_some()
    }

    /// Capture the current link record.
    pub fn capture_link(&mut self) {
        if let Some(linked) = self.body.linked() {
            self.previous_link = Some(linked);
        }
    }

    /// Link count, falling back to the previous link record.
    #[must_use]
    pub fn linkcount(&self) -> i32 {
        if let Some(linked) = self.body.linked() {
            linked.link_count
        } else {
            self.previous_link.as_ref().map_or(0, |link| link.link_count)
        }
    }

    /// Local minimum bounds.
    #[must_use]
    pub fn mins(&self) -> Vec3 {
        self.body.read().bounds.min
    }

    /// Write the local minimum bounds.
    pub fn set_mins(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.bounds.min = value;
        self.body.write(state);
    }

    /// Local maximum bounds.
    #[must_use]
    pub fn maxs(&self) -> Vec3 {
        self.body.read().bounds.max
    }

    /// Write the local maximum bounds.
    pub fn set_maxs(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.bounds.max = value;
        self.body.write(state);
    }

    /// Current collision origin, from the temporary view when active.
    #[must_use]
    pub fn current_origin(&self) -> Vec3 {
        self.current_origin_view.unwrap_or_else(|| self.body.read().origin)
    }

    /// Write the current collision origin.
    pub fn set_current_origin(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.origin = value;
        self.body.write(state);
        if let Some(view) = self.current_origin_view.as_mut() {
            *view = value;
        }
    }

    /// Run a call under a temporary snapped origin view.
    ///
    /// `ClientThink` links a snapped source origin while `ps.origin`
    /// retains movement precision.
    pub fn with_current_origin<R>(&mut self, origin: Vec3, call: impl FnOnce() -> R) -> R {
        let previous = self.current_origin_view;
        self.current_origin_view = Some(origin);
        let result = call();
        self.current_origin_view = previous;
        result
    }

    /// Current collision angles.
    #[must_use]
    pub fn current_angles(&self) -> Vec3 {
        self.body.read().angles
    }

    /// Write the current collision angles.
    pub fn set_current_angles(&mut self, value: Vec3) {
        let mut state = self.body.read();
        state.angles = value;
        self.body.write(state);
    }

    /// Capture private save words.
    pub fn capture_private_state(&self) -> Result<PrivateEntityState, Q3BaseError> {
        if self.current_origin_view.is_some() {
            return Err(Q3BaseError::Invalid(
                "Cannot save inside a Q3 temporary origin view".to_string(),
            ));
        }
        Ok(PrivateEntityState {
            previous_link: self.previous_link.clone(),
            abs_min_override: self.abs_min_override,
            abs_max_override: self.abs_max_override,
        })
    }

    /// Restore private save words.
    pub fn restore_private_state(&mut self, state: &PrivateEntityState) -> Result<(), Q3BaseError> {
        if self.current_origin_view.is_some() {
            return Err(Q3BaseError::Invalid(
                "Cannot restore inside a Q3 temporary origin view".to_string(),
            ));
        }
        self.previous_link = state.previous_link.clone();
        self.abs_min_override = state.abs_min_override;
        self.abs_max_override = state.abs_max_override;
        Ok(())
    }

    /// Clear absolute-bounds overrides.
    pub fn clear_bounds_overrides(&mut self) {
        self.abs_min_override = None;
        self.abs_max_override = None;
    }

    /// Override the absolute minimum bounds.
    pub fn set_absmin(&mut self, value: Vec3) {
        self.abs_min_override = Some(value);
    }

    /// Override the absolute maximum bounds.
    pub fn set_absmax(&mut self, value: Vec3) {
        self.abs_max_override = Some(value);
    }

    /// Absolute minimum bounds.
    #[must_use]
    pub fn absmin(&self) -> Vec3 {
        self.abs_min_override.unwrap_or_else(|| {
            self.body
                .linked()
                .map(|linked| linked.absolute_bounds.min)
                .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.min))
                .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
        })
    }

    /// Absolute maximum bounds.
    #[must_use]
    pub fn absmax(&self) -> Vec3 {
        self.abs_max_override.unwrap_or_else(|| {
            self.body
                .linked()
                .map(|linked| linked.absolute_bounds.max)
                .or_else(|| self.previous_link.as_ref().map(|link| link.absolute_bounds.max))
                .unwrap_or_else(|| vec3(0.0, 0.0, 0.0))
        })
    }
}

/// Shared entity state words (`SharedEntityState`).
pub type SharedEntityState = EntityState;

/// Entity with shared state words (`SharedEntity`).
pub trait SharedEntity {
    /// Entity state words.
    fn entity_state(&self) -> &EntityState;
    /// Shared collision metadata.
    fn shared(&self) -> &EntityShared;
}
