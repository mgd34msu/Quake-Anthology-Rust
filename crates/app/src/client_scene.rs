//! CLIENT-owned snapshot values become copied entries in the shared scene list.
use crate::model_assets::{
    InlineBinding, ModelBinding, ModelCatalog, ModelMaterialPolicy, ModelProfile,
    ModelRegistrationId,
};
use crate::snapshots::{GunAnimation, Interpolation};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    math::{angle_vectors, lerp_angle},
    primitives::{
        ClientId, EntityAppearance, IndexSpan, LoopOrigin, ModelRef, NativeAppearanceTail,
        PlayerState, ResourceNamespace, RuleSetId, Vec3, ViewWeapon,
    },
};
use qa_gameplay::rules::appearance::{
    AliasPose, AliasRoll, AppearanceCapabilities, AppearancePolicy, WeaponInterpolation,
};
use qa_network::{
    projection::{EntityProjection, EntityRecord, PlayerProjection},
    snapshots::{Frame as SnapshotFrame, ReceivedFrame},
};
use qa_render::{
    Assets, Frame, Refdef, SceneEntity, SceneModel,
    world::{WorldId, WorldPresentationId},
};

#[derive(Clone, Copy)]
enum ClientModel {
    Mesh(ModelRegistrationId),
    Inline {
        binding: InlineBinding,
        presentation: WorldPresentationId,
    },
}

#[derive(Clone, Copy)]
enum SceneBinding {
    Mesh(ModelBinding),
    Inline {
        binding: InlineBinding,
        presentation: WorldPresentationId,
    },
}

/// Only derived numeric asset bindings live here. Accepted entity values and
/// history remain in the connection's existing snapshot ring.
pub struct ClientModelBindings {
    client: ClientId,
    range: IndexSpan,
    profile: Option<ModelProfile>,
    world: Option<(WorldId, WorldPresentationId)>,
    registrations: Box<[Option<ClientModel>]>,
}
impl ClientModelBindings {
    pub fn load(client: ClientId, layout: ConfigLayout) -> Self {
        let range = layout.resources[0];
        Self {
            client,
            range,
            profile: None,
            world: None,
            registrations: vec![None; range.count as usize].into_boxed_slice(),
        }
    }

    /// Disconnect/rebinding clears all weak references into the old catalog.
    pub fn reset(&mut self) {
        self.profile = None;
        self.world = None;
        self.registrations.fill(None);
    }

    pub(crate) fn client(&self) -> ClientId {
        self.client
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
        self.registrations[ordinal] = None;
        let Some(row) = configs.config(index) else {
            return false;
        };
        if row.text.is_empty() || ordinal == 0 {
            return true;
        }
        let Ok(path) = std::str::from_utf8(row.text) else {
            return false;
        };
        if let Some(index) = path.strip_prefix('*') {
            let Some((world, presentation)) = self.world else {
                return false;
            };
            let Some(binding) = index
                .parse::<u32>()
                .ok()
                .and_then(|index| models.resolve_inline(world, index))
            else {
                return false;
            };
            self.registrations[ordinal] = Some(ClientModel::Inline {
                binding,
                presentation,
            });
            return true;
        }
        let Some(registration) = self
            .profile
            .and_then(|profile| models.registration(path, profile))
        else {
            return false;
        };
        self.registrations[ordinal] = Some(ClientModel::Mesh(registration));
        true
    }

    pub fn bind_all(
        &mut self,
        configs: &ConfigStrings,
        models: &ModelCatalog,
        profile: ModelProfile,
        world: Option<(WorldId, WorldPresentationId)>,
    ) -> u32 {
        self.reset();
        self.profile = Some(profile);
        self.world = world;
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
        match self.resolve_scene(reference, skin, models, policy)? {
            SceneBinding::Mesh(binding) => Some(binding),
            SceneBinding::Inline { .. } => None,
        }
    }

    fn resolve_scene(
        &self,
        reference: ModelRef,
        skin: i32,
        models: &ModelCatalog,
        policy: ModelMaterialPolicy,
    ) -> Option<SceneBinding> {
        if reference.namespace != ResourceNamespace::Client(self.client) {
            return None;
        }
        let registration = self
            .registrations
            .get(reference.ordinal as usize)
            .copied()
            .flatten()?;
        match registration {
            ClientModel::Mesh(registration) => models
                .binding(registration, u32::try_from(skin).unwrap_or(0), policy)
                .map(SceneBinding::Mesh),
            ClientModel::Inline {
                binding,
                presentation,
            } => Some(SceneBinding::Inline {
                binding,
                presentation,
            }),
        }
    }
}

/// A view of the connection's sole snapshot ring, shared by scene and audio.
/// No asset registration, SERVER state or native child memory is needed.
#[derive(Clone, Copy)]
pub struct ClientEntities<'a> {
    pub client: ClientId,
    /// Accepted/predicted CLIENT state, never the SERVER player array.
    pub player: Option<&'a PlayerState>,
    pub current: Option<ReceivedFrame<'a>>,
    pub previous: Option<ReceivedFrame<'a>>,
    pub projection: &'a EntityProjection,
    pub appearance: AppearancePolicy,
    pub alias_roll: AliasRoll,
    pub interpolation: Interpolation,
}

pub struct PresentedEntity {
    pub record: EntityRecord,
    pub previous: Option<EntityRecord>,
    pub origin: Vec3,
    pub angles: Vec3,
    pub capabilities: AppearanceCapabilities,
}
impl ClientEntities<'_> {
    /// Native frame numbers, not shader time, determine whether Q2 has a
    /// usable predecessor. Other protocols retain their existing policy.
    fn adjacent(&self, current: u32, previous: u32) -> bool {
        self.appearance.adjacent(current, previous)
    }

    fn decode(&self, words: &[u32]) -> Option<EntityRecord> {
        let mut record = EntityRecord::default();
        self.projection
            .apply(words, &mut record, ResourceNamespace::Client(self.client))
            .then_some(record)
    }

    fn row<'a>(frame: ReceivedFrame<'a>, number: u32) -> Option<(&'a [u32], bool)> {
        macro_rules! find {
            ($frame:expr) => {{
                let frame = $frame;
                let index = frame
                    .entities
                    .binary_search_by_key(&number, |row| row.number)
                    .ok()?;
                Some((&frame.entities[index].words[..], frame.qw_player(number)))
            }};
        }
        match frame {
            ReceivedFrame::NetQuake(frame) => find!(frame),
            ReceivedFrame::QuakeWorld(frame) => find!(frame),
            ReceivedFrame::Quake2(frame) => find!(frame),
            ReceivedFrame::Quake2Repro(frame) => find!(frame),
            ReceivedFrame::Quake3(frame) => find!(frame),
        }
    }

    fn present(
        &self,
        number: u32,
        record: EntityRecord,
        qw_player: bool,
        interpolate: bool,
    ) -> PresentedEntity {
        let capabilities = self.appearance.project(&record.appearance);
        let previous = self
            .previous
            .filter(|_| interpolate)
            .filter(|previous| {
                self.current.is_some_and(|current| {
                    std::mem::discriminant(&current) == std::mem::discriminant(previous)
                        && self.adjacent(current.sequence(), previous.sequence())
                })
            })
            .and_then(|previous| Self::row(previous, number))
            .and_then(|(words, _)| self.decode(words))
            .filter(|old| {
                self.appearance.can_interpolate(
                    &old.appearance,
                    &record.appearance,
                    old.position,
                    record.position,
                )
            });
        let fraction = self.interpolation.fraction;
        let origin = if capabilities.frame_lerp {
            record.position
        } else {
            previous.map_or(record.position, |old| {
                old.position.lerp(record.position, fraction)
            })
        };
        let angles = previous.map_or(record.angles, |old| {
            Vec3(std::array::from_fn(|axis| {
                lerp_angle(old.angles.0[axis], record.angles.0[axis], fraction)
            }))
        });
        let angles =
            self.appearance
                .body_angles(angles, record.velocity, qw_player, self.alias_roll);
        PresentedEntity {
            record,
            previous,
            origin,
            angles,
            capabilities,
        }
    }

    /// Walk the sole accepted frame in its native entity order. A malformed
    /// row remains visible to the caller as a rejected admission, not a copy.
    pub fn visit(&self, visit: impl FnMut(u32, Option<PresentedEntity>)) {
        self.visit_rows(None, visit);
    }

    fn player_record(&self) -> Option<(u32, EntityRecord)> {
        let Some(ReceivedFrame::Quake3(_)) = self.current else {
            return None;
        };
        let player = self.player?;
        let native = player.native_for(
            RuleSetId::Quake3,
            Some(ResourceNamespace::Client(self.client)),
        )?;
        Some((
            native.client_number?,
            EntityRecord {
                position: player.body.position,
                velocity: player.body.velocity,
                angles: player.view_angles,
                appearance: EntityAppearance {
                    source: native.source,
                    loop_sound: player.loop_sound,
                    native: NativeAppearanceTail {
                        entity_type: 1, // Original Q3 ET_PLAYER.
                        ..Default::default()
                    },
                    ..Default::default()
                },
                ..Default::default()
            },
        ))
    }

    /// Q3 adds the predicted player before packet entities. A nonzero packet
    /// loop replaces that sound; a zero packet loop only updates its origin.
    pub(crate) fn visit_loops(&self, visit: impl FnMut(u32, Option<PresentedEntity>)) {
        self.visit_rows(self.player_record(), visit);
    }

    fn visit_rows(
        &self,
        supplement: Option<(u32, EntityRecord)>,
        mut visit: impl FnMut(u32, Option<PresentedEntity>),
    ) {
        fn rows<const P: usize, const E: usize>(
            entities: &ClientEntities<'_>,
            frame: SnapshotFrame<'_, P, E>,
            mut supplement: Option<(u32, EntityRecord)>,
            visit: &mut impl FnMut(u32, Option<PresentedEntity>),
        ) {
            for row in frame.entities {
                if let Some((number, record)) =
                    supplement.filter(|(number, _)| *number < row.number)
                {
                    supplement = None;
                    visit(number, Some(entities.present(number, record, false, false)));
                }
                let predicted = supplement
                    .filter(|(number, _)| *number == row.number)
                    .map(|(_, record)| record);
                if predicted.is_some() {
                    supplement = None;
                }
                visit(
                    row.number,
                    entities.decode(&row.words).map(|mut record| {
                        if record.appearance.loop_sound.is_none()
                            && let Some(predicted) = predicted
                        {
                            record.appearance.loop_sound = predicted.appearance.loop_sound;
                        }
                        entities.present(row.number, record, frame.qw_player(row.number), true)
                    }),
                );
            }
            if let Some((number, record)) = supplement {
                visit(number, Some(entities.present(number, record, false, false)));
            }
        }
        match self.current {
            Some(ReceivedFrame::NetQuake(frame)) => rows(self, frame, supplement, &mut visit),
            Some(ReceivedFrame::QuakeWorld(frame)) => rows(self, frame, supplement, &mut visit),
            Some(ReceivedFrame::Quake2(frame)) => rows(self, frame, supplement, &mut visit),
            Some(ReceivedFrame::Quake2Repro(frame)) => rows(self, frame, supplement, &mut visit),
            Some(ReceivedFrame::Quake3(frame)) => rows(self, frame, supplement, &mut visit),
            None => {}
        }
    }

    /// Numeric lookup in accepted CLIENT rows. The pose uses the same native
    /// discontinuity and interpolation path as the copied scene entities.
    pub fn get(&self, number: u32) -> Option<PresentedEntity> {
        if let Some((words, qw_player)) = Self::row(self.current?, number) {
            return Some(self.present(number, self.decode(words)?, qw_player, true));
        }
        self.player_record()
            .filter(|(native, _)| *native == number)
            .map(|(_, record)| self.present(number, record, false, false))
    }

    /// NQ/QW/Q2 reserve ordinal zero for the world and never receive a pose
    /// for it. Q3 client/entity zero is a real row and must be resolved.
    pub fn sound_origin(&self, number: u32, origin: Option<LoopOrigin>) -> Option<Vec3> {
        if number == 0 && self.appearance.world_zero() {
            Some(Vec3::default())
        } else {
            self.get(number).map(|entity| match origin {
                Some(LoopOrigin::Accepted) => entity.record.position,
                None | Some(LoopOrigin::Presented) => entity.origin,
            })
        }
    }
}

/// The frame borrows only accepted CLIENT data. No native memory or SERVER
/// entity table may be observed by the renderer.
pub struct ClientSceneInput<'a> {
    pub client: ClientId,
    pub current: Option<ReceivedFrame<'a>>,
    pub previous: Option<ReceivedFrame<'a>>,
    pub projection: &'a EntityProjection,
    pub appearance: AppearancePolicy,
    pub player_projection: &'a PlayerProjection,
    pub bindings: &'a ClientModelBindings,
    pub models: &'a ModelCatalog,
    /// Owned signon values outlive the source edict and snapshot ring slots.
    pub static_entities: &'a [EntityRecord],
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
    pub fn entities(&self) -> ClientEntities<'_> {
        ClientEntities {
            client: self.client,
            player: Some(self.player),
            current: self.current,
            previous: self.previous,
            projection: self.projection,
            appearance: self.appearance,
            alias_roll: self.alias_roll,
            interpolation: self.interpolation,
        }
    }

    /// Accepted snapshots already use common hidden-area polarity.
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
        output[..count].copy_from_slice(&areas[..count]);
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
        let fraction = self.interpolation.fraction;
        self.entities().visit(|number, pose| {
            counts.entities += 1;
            let Some(pose) = pose else {
                counts.rejected += 1;
                return;
            };
            self.append_presented(
                pose,
                Some(number),
                fraction,
                frame,
                assets,
                software,
                &mut counts,
            );
        });
        for &record in self.static_entities {
            counts.entities += 1;
            self.append_presented(
                PresentedEntity {
                    record,
                    previous: None,
                    origin: record.position,
                    angles: record.angles,
                    capabilities: self.appearance.project(&record.appearance),
                },
                None,
                1.0,
                frame,
                assets,
                software,
                &mut counts,
            );
        }
        self.append_weapon(frame, assets, refdef, software, &mut counts);
        counts
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "Dynamic and retained static values share one scene submission path."
    )]
    fn append_presented(
        &self,
        pose: PresentedEntity,
        number: Option<u32>,
        fraction: f32,
        frame: &mut Frame,
        assets: &Assets,
        software: bool,
        counts: &mut SceneCounts,
    ) {
        let record = pose.record;
        let capabilities = pose.capabilities;
        counts.unsupported += u32::from(capabilities.unsupported != 0);
        if !capabilities.drawable
            || number.is_some_and(|number| self.view_entity == Some(number))
            || capabilities.viewer_model
        {
            return;
        }
        let previous = pose.previous;
        if capabilities.frame_lerp
            && record.appearance.old_origin != record.position
            && fraction != 1.0
        {
            counts.unsupported += 1;
        }
        for (slot, reference) in record.appearance.models.iter().enumerate() {
            let Some(reference) = *reference else {
                continue;
            };
            let Some(binding) = self.bindings.resolve_scene(
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
            let mut entity = SceneEntity {
                origin: pose.origin,
                non_normalized_axes: capabilities.scale != 1.0,
                color: [255, 255, 255, (capabilities.alpha * 255.0) as u8],
                depth_hack: capabilities.depth_hack,
                ..SceneEntity::default()
            };
            let angles = match binding {
                SceneBinding::Inline {
                    binding,
                    presentation,
                } => {
                    entity.model = SceneModel::World {
                        world: binding.world,
                        presentation,
                        surfaces: binding.surfaces,
                    };
                    pose.angles
                }
                SceneBinding::Mesh(binding) => {
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
                    let angles = binding.pose.angles(pose.angles, software);
                    entity.model = binding.model.into();
                    entity.material = Some(binding.material);
                    entity.frame = current_frame;
                    entity.old_frame = old_frame;
                    entity.back_lerp = 1.0 - fraction;
                    entity.lighting = Some(native_lighting(binding.pose, angles));
                    angles
                }
            };
            entity.axes = angle_vectors(angles)
                .axes()
                .map(|axis| axis * capabilities.scale);
            let added = frame.add_entity(entity);
            counts.models += u32::from(added);
            counts.rejected += u32::from(!added);
        }
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
        let animation_policy = self.appearance.weapon_interpolation();
        let previous = if animation_policy != WeaponInterpolation::None {
            self.previous
                .filter(|old| {
                    self.current.is_some_and(|current| {
                        self.entities().adjacent(current.sequence(), old.sequence())
                    })
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
        let angles = binding
            .pose
            .angles(self.player.view_angles + angles, software);
        let current_frame = native_frame(weapon.frame, model.frame_count());
        let (old_frame, back_lerp) =
            if animation_policy == WeaponInterpolation::Rate && previous.is_some() {
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
            } else if animation_policy == WeaponInterpolation::Snapshot {
                (
                    previous
                        .filter(|_| weapon.frame != 0)
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
            model: binding.model.into(),
            material: Some(binding.material),
            origin: refdef.origin + offset + (weapon.offset - offset) * fraction,
            axes: angle_vectors(angles).axes(),
            frame: current_frame,
            old_frame,
            back_lerp,
            depth_hack: true,
            lighting: Some(native_lighting(binding.pose, angles)),
            ..SceneEntity::default()
        };
        let added = frame.add_entity(entity);
        counts.models += u32::from(added);
        counts.rejected += u32::from(!added);
    }

    fn weapon(&self, frame: ReceivedFrame<'_>) -> Option<ViewWeapon> {
        self.player_projection.view_weapon(
            frame.player_words()?,
            ResourceNamespace::Client(self.client),
        )
    }

    fn player_event_interpolation(&self) -> bool {
        if !self.appearance.player_event_check() {
            return true;
        }
        let (Some(current), Some(number)) = (self.current, self.view_entity) else {
            return true;
        };
        let Some((words, _)) = ClientEntities::row(current, number) else {
            return true;
        };
        self.entities()
            .decode(words)
            .is_some_and(|entity| !self.appearance.is_interpolation_event(&entity.appearance))
    }
}

/// Native alias renderers fall back to frame zero for invalid frame numbers.
fn native_frame(frame: i32, count: u32) -> u32 {
    u32::try_from(frame)
        .ok()
        .filter(|frame| *frame < count)
        .unwrap_or(0)
}

fn native_lighting(pose: AliasPose, angles: Vec3) -> qa_render::stage::EntityLighting {
    let mut lighting = qa_render::stage::EntityLighting::default();
    if let Some(direction) = pose.light_direction(angles) {
        lighting.direction = direction;
    }
    lighting
}
