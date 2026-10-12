//! CLIENT-owned snapshot values become copied entries in the shared scene list.
use crate::model_assets::{ModelBinding, ModelCatalog, ModelMaterialPolicy};
use crate::snapshots::{GunAnimation, Interpolation};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    math::{angle_vectors, lerp_angle},
    primitives::{
        ClientId, IndexSpan, ModelRef, NameId, PlayerState, ResourceNamespace, RuleSetId, Vec3,
        ViewWeapon,
    },
};
use qa_gameplay::rules::appearance::{AliasRoll, alias_pose_angles};
use qa_network::{
    projection::{EntityProjection, EntityRecord, PlayerProjection},
    snapshots::{Entity, Frame as SnapshotFrame, ReceivedFrame},
};
use qa_render::{Assets, Frame, Refdef, SceneEntity};

/// Only derived numeric asset bindings live here. Accepted entity values and
/// history remain in the connection's existing snapshot ring.
pub struct ClientModelBindings {
    client: ClientId,
    range: IndexSpan,
    names: Box<[Option<NameId>]>,
}
impl ClientModelBindings {
    pub fn load(client: ClientId, layout: ConfigLayout) -> Self {
        let range = layout.resources[0];
        Self {
            client,
            range,
            names: vec![None; range.count as usize].into_boxed_slice(),
        }
    }

    /// Disconnect/rebinding clears all weak references into the old catalog.
    pub fn reset(&mut self) {
        self.names.fill(None);
    }

    /// Text decoding and canonical name lookup happen at configuration changes,
    /// never while walking visible entities each frame.
    pub fn config_changed(
        &mut self,
        index: usize,
        configs: &ConfigStrings,
        models: &ModelCatalog,
    ) -> bool {
        if !self.range.contains(index) {
            return true;
        }
        let ordinal = index - self.range.first as usize;
        self.names[ordinal] = None;
        let Some(row) = configs.config(index) else {
            return false;
        };
        if row.text.is_empty() || ordinal == 0 {
            return true;
        }
        let Ok(path) = std::str::from_utf8(row.text) else {
            return false;
        };
        let Some(name) = models.name_for_path(path) else {
            return false;
        };
        self.names[ordinal] = Some(name);
        true
    }

    pub fn bind_all(&mut self, configs: &ConfigStrings, models: &ModelCatalog) -> u32 {
        self.reset();
        self.range.range().fold(0, |missing, index| {
            missing + u32::from(!self.config_changed(index, configs, models))
        })
    }

    pub fn resolve(
        &self,
        reference: ModelRef,
        skin: i32,
        models: &ModelCatalog,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelBinding> {
        if reference.namespace != ResourceNamespace::Client(self.client) {
            return None;
        }
        let name = self
            .names
            .get(reference.ordinal as usize)
            .copied()
            .flatten()?;
        models.resolve_name_with(name, u32::try_from(skin).unwrap_or(0), policy)
    }
}

/// The frame borrows only accepted CLIENT data. No native memory or SERVER
/// entity table may be observed by the renderer.
pub struct ClientSceneInput<'a> {
    pub client: ClientId,
    pub current: Option<ReceivedFrame<'a>>,
    pub previous: Option<ReceivedFrame<'a>>,
    pub projection: &'a EntityProjection,
    pub player_projection: &'a PlayerProjection,
    pub bindings: &'a ClientModelBindings,
    pub models: &'a ModelCatalog,
    pub player: &'a PlayerState,
    pub alias_roll: AliasRoll,
    /// Snapshot-domain time is independent of shader/refdef time.
    pub interpolation: Interpolation,
    pub gun_animation: Option<GunAnimation>,
    pub gun_previous: Option<ReceivedFrame<'a>>,
    /// Negotiated native view entity; common client/entity slots cannot supply it.
    pub view_entity: Option<u32>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneCounts {
    pub entities: u32,
    pub models: u32,
    pub missing_models: u32,
    pub unsupported: u32,
    pub rejected: u32,
}
impl std::ops::AddAssign for SceneCounts {
    fn add_assign(&mut self, rhs: Self) {
        self.entities += rhs.entities;
        self.models += rhs.models;
        self.missing_models += rhs.missing_models;
        self.unsupported += rhs.unsupported;
        self.rejected += rhs.rejected;
    }
}

impl ClientSceneInput<'_> {
    /// Wire area-bit polarity is converted once at the CLIENT scene boundary.
    /// The world query and copied render command borrow the same hidden bits.
    pub fn hidden_areas<'a>(&self, output: &'a mut [u8]) -> (&'a [u8], bool) {
        let areas = match self.current {
            Some(ReceivedFrame::NetQuake(frame)) => frame.areas,
            Some(ReceivedFrame::QuakeWorld(frame)) => frame.areas,
            Some(ReceivedFrame::Quake2(frame)) => frame.areas,
            Some(ReceivedFrame::Quake2Repro(frame)) => frame.areas,
            Some(ReceivedFrame::Quake3(frame)) => frame.areas,
            None => &[],
        };
        let count = areas.len().min(output.len());
        let invert = matches!(
            self.projection.rule_set(),
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        );
        for (to, from) in output[..count].iter_mut().zip(&areas[..count]) {
            *to = if invert { !*from } else { *from };
        }
        (&output[..count], count != areas.len())
    }

    pub fn append(
        &self,
        frame: &mut Frame,
        assets: &Assets,
        refdef: &Refdef,
        software: bool,
    ) -> SceneCounts {
        let mut counts = SceneCounts::default();
        if let Some(current) = self.current {
            macro_rules! append {
                ($variant:ident, $current:expr) => {{
                    let previous = match self.previous {
                        Some(ReceivedFrame::$variant(previous)) => Some(previous),
                        _ => None,
                    };
                    self.append_entities($current, previous, frame, assets, software, &mut counts);
                }};
            }
            match current {
                ReceivedFrame::NetQuake(current) => append!(NetQuake, current),
                ReceivedFrame::QuakeWorld(current) => append!(QuakeWorld, current),
                ReceivedFrame::Quake2(current) => append!(Quake2, current),
                ReceivedFrame::Quake2Repro(current) => append!(Quake2Repro, current),
                ReceivedFrame::Quake3(current) => append!(Quake3, current),
            }
        }
        self.append_weapon(frame, assets, refdef, software, &mut counts);
        counts
    }

    fn append_entities<const P: usize, const E: usize>(
        &self,
        current: SnapshotFrame<'_, P, E>,
        previous: Option<SnapshotFrame<'_, P, E>>,
        frame: &mut Frame,
        assets: &Assets,
        software: bool,
        counts: &mut SceneCounts,
    ) {
        let previous =
            previous.filter(|previous| self.adjacent(current.sequence, previous.sequence));
        let fraction = self.interpolation.fraction;
        for entity in current.entities {
            counts.entities += 1;
            let Some(record) = self.decode(entity) else {
                counts.rejected += 1;
                continue;
            };
            let capabilities = qa_gameplay::rules::appearance::project(&record.appearance);
            counts.unsupported += u32::from(capabilities.unsupported != 0);
            if !capabilities.drawable
                || self.view_entity == Some(entity.number)
                || capabilities.viewer_model
            {
                continue;
            }
            let previous = previous.and_then(|previous| {
                let index = previous
                    .entities
                    .binary_search_by_key(&entity.number, |entity| entity.number)
                    .ok()?;
                let old = self.decode(&previous.entities[index])?;
                qa_gameplay::rules::appearance::can_interpolate(
                    &old.appearance,
                    &record.appearance,
                    old.position,
                    record.position,
                )
                .then_some(old)
            });
            let position = if capabilities.frame_lerp {
                if record.appearance.old_origin != record.position && fraction != 1.0 {
                    counts.unsupported += 1;
                }
                record.position
            } else {
                previous.map_or(record.position, |old| {
                    old.position.lerp(record.position, fraction)
                })
            };
            for (slot, reference) in record.appearance.models.iter().enumerate() {
                let Some(reference) = *reference else {
                    continue;
                };
                let Some(binding) = self.bindings.resolve(
                    reference,
                    record.appearance.skin,
                    self.models,
                    ModelMaterialPolicy {
                        translucent: capabilities.alpha != 1.0,
                        fullbright: capabilities.fullbright,
                    },
                ) else {
                    counts.missing_models += 1;
                    continue;
                };
                let Some(model) = assets.model(binding.model) else {
                    counts.missing_models += 1;
                    continue;
                };
                let current_frame = native_frame(record.appearance.frame, model.frame_count());
                let old_frame = if capabilities.state_old_frame {
                    native_frame(record.appearance.native.old_frame, model.frame_count())
                } else {
                    previous
                        .filter(|old| old.appearance.models[slot] == Some(reference))
                        .map_or(current_frame, |old| {
                            native_frame(old.appearance.frame, model.frame_count())
                        })
                };
                let angles = previous.map_or(record.angles, |old| {
                    Vec3(std::array::from_fn(|axis| {
                        lerp_angle(old.angles.0[axis], record.angles.0[axis], fraction)
                    }))
                });
                let angles = alias_pose_angles(
                    binding.rules,
                    angles,
                    record.velocity,
                    current.qw_player(entity.number),
                    software,
                    self.alias_roll,
                );
                let axes = angle_vectors(angles)
                    .axes()
                    .map(|axis| axis * capabilities.scale);
                let entity = SceneEntity {
                    model: binding.model,
                    material: Some(binding.material),
                    origin: position,
                    axes,
                    non_normalized_axes: capabilities.scale != 1.0,
                    color: [255, 255, 255, (capabilities.alpha * 255.0) as u8],
                    frame: current_frame,
                    old_frame,
                    back_lerp: 1.0 - fraction,
                    depth_hack: capabilities.depth_hack,
                    lighting: Some(native_lighting(binding.rules, angles)),
                    ..SceneEntity::default()
                };
                let added = frame.add_entity(entity);
                counts.models += u32::from(added);
                counts.rejected += u32::from(!added);
            }
        }
    }

    fn decode<const E: usize>(&self, entity: &Entity<E>) -> Option<EntityRecord> {
        let mut record = EntityRecord::default();
        self.projection
            .apply(
                &entity.words,
                &mut record,
                ResourceNamespace::Client(self.client),
            )
            .then_some(record)
    }

    fn append_weapon(
        &self,
        frame: &mut Frame,
        assets: &Assets,
        refdef: &Refdef,
        software: bool,
        counts: &mut SceneCounts,
    ) {
        let weapon = self
            .current
            .and_then(|frame| self.weapon(frame))
            .unwrap_or(self.player.view_weapon);
        let Some(reference) = weapon.model else {
            return;
        };
        let Some(binding) = self.bindings.resolve(
            reference,
            weapon.skin,
            self.models,
            ModelMaterialPolicy::default(),
        ) else {
            counts.missing_models += 1;
            return;
        };
        let Some(model) = assets.model(binding.model) else {
            counts.missing_models += 1;
            return;
        };
        let source = self.projection.rule_set();
        let native_q2 = matches!(source, RuleSetId::Quake2 | RuleSetId::Quake2Rerelease);
        let previous = if native_q2 {
            self.previous
                .filter(|old| {
                    self.current
                        .is_some_and(|current| self.adjacent(current.sequence(), old.sequence()))
                })
                .filter(|old| {
                    self.current
                        .and_then(ReceivedFrame::player_words)
                        .zip(old.player_words())
                        .is_some_and(|(current, previous)| {
                            self.player_projection
                                .can_interpolate_player(previous, current)
                        })
                })
                .filter(|_| self.player_event_interpolation())
                .and_then(|old| self.weapon(old))
        } else {
            None
        };
        let offset = previous.map_or(weapon.offset, |old| old.offset);
        let fraction = self.interpolation.fraction;
        let angles = previous.map_or(weapon.angles, |old| {
            Vec3(std::array::from_fn(|axis| {
                lerp_angle(old.angles.0[axis], weapon.angles.0[axis], fraction)
            }))
        });
        let angles = alias_pose_angles(
            binding.rules,
            self.player.view_angles + angles,
            Vec3::default(),
            false,
            software,
            self.alias_roll,
        );
        let current_frame = native_frame(weapon.frame, model.frame_count());
        let (old_frame, back_lerp) = if source == RuleSetId::Quake2Rerelease && previous.is_some() {
            let back = self.gun_animation.map_or(0.0, |animation| {
                self.interpolation
                    .gun_back_lerp(animation.changed_at, weapon.rate)
            });
            let old = self
                .gun_previous
                .and_then(|frame| self.weapon(frame))
                .filter(|old| old.model == weapon.model);
            if old.is_none()
                && back != 0.0
                && self
                    .gun_animation
                    .is_some_and(|animation| animation.previous.is_some())
            {
                counts.unsupported += 1;
            }
            (
                old.map_or(current_frame, |old| {
                    native_frame(old.frame, model.frame_count())
                }),
                back,
            )
        } else if source == RuleSetId::Quake2 && weapon.frame != 0 {
            (
                previous
                    .filter(|old| old.model == weapon.model)
                    .map_or(current_frame, |old| {
                        native_frame(old.frame, model.frame_count())
                    }),
                1.0 - fraction,
            )
        } else {
            (current_frame, 0.0)
        };
        let entity = SceneEntity {
            model: binding.model,
            material: Some(binding.material),
            origin: refdef.origin + offset + (weapon.offset - offset) * fraction,
            axes: angle_vectors(angles).axes(),
            frame: current_frame,
            old_frame,
            back_lerp,
            depth_hack: true,
            lighting: Some(native_lighting(binding.rules, angles)),
            ..SceneEntity::default()
        };
        let added = frame.add_entity(entity);
        counts.models += u32::from(added);
        counts.rejected += u32::from(!added);
    }

    fn adjacent(&self, current: u32, previous: u32) -> bool {
        !matches!(
            self.projection.rule_set(),
            RuleSetId::Quake2 | RuleSetId::Quake2Rerelease
        ) || current.wrapping_sub(previous) == 1
    }

    fn weapon(&self, frame: ReceivedFrame<'_>) -> Option<ViewWeapon> {
        self.player_projection.view_weapon(
            frame.player_words()?,
            ResourceNamespace::Client(self.client),
        )
    }

    fn player_event_interpolation(&self) -> bool {
        let (Some(ReceivedFrame::Quake2Repro(current)), Some(number)) =
            (self.current, self.view_entity)
        else {
            return true;
        };
        let Ok(index) = current
            .entities
            .binary_search_by_key(&number, |entity| entity.number)
        else {
            return true;
        };
        self.decode(&current.entities[index]).is_some_and(|entity| {
            qa_gameplay::rules::appearance::can_interpolate_player_event(&entity.appearance)
        })
    }
}

/// Native alias renderers fall back to frame zero for invalid frame numbers.
fn native_frame(frame: i32, count: u32) -> u32 {
    u32::try_from(frame)
        .ok()
        .filter(|frame| *frame < count)
        .unwrap_or(0)
}

fn native_lighting(source: RuleSetId, angles: Vec3) -> qa_render::stage::EntityLighting {
    let mut lighting = qa_render::stage::EntityLighting::default();
    if let Some(direction) = qa_gameplay::rules::appearance::alias_light_direction(source, angles) {
        lighting.direction = direction;
    }
    lighting
}
