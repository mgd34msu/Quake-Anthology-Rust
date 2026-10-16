//! CLIENT-owned snapshot values become copied entries in the shared scene list.
use crate::model_assets::{
    InlineBinding, ModelBinding, ModelBindings, ModelCatalog, ModelMaterialPolicy, ModelProfile,
    ModelRegistrationId,
};
use crate::snapshots::{GunAnimation, Interpolation};
use qa_core::{
    configstrings::{ConfigLayout, ConfigStrings},
    math::{angle_vectors, lerp_angle},
    primitives::{
        ClientId, EntityAppearance, GeometryId, IndexSpan, LoopOrigin, ModelRef,
        NativeAppearanceTail, PlayerState, ResourceNamespace, RuleSetId, Vec3, ViewWeapon,
    },
};
use qa_gameplay::rules::appearance::{
    AliasPose, AliasRoll, AppearanceCapabilities, AppearancePolicy, ModelFeatures, ModelInput,
    WeaponInterpolation,
};
use qa_network::{
    projection::{EntityProjection, EntityRecord, PlayerProjection},
    snapshots::{Frame as SnapshotFrame, ReceivedFrame},
};
use qa_render::{
    Assets, Frame, Refdef, SceneEntity, SceneModel,
    world::{WorldId, WorldPresentationId},
};

fn inline_model_ordinal(name: &[u8]) -> Option<u32> {
    std::str::from_utf8(name.strip_prefix(b"*")?)
        .ok()?
        .parse()
        .ok()
}

#[derive(Clone, Copy)]
enum ClientModel {
    Mesh(ModelRegistrationId),
    Inline {
        binding: InlineBinding,
        presentation: Option<WorldPresentationId>,
        collision: Option<GeometryId>,
    },
}

#[derive(Clone, Copy)]
enum ModelCapture {
    Bound(ClientModel),
    Registered {
        handle: i32,
        presentation: Option<WorldPresentationId>,
    },
}
#[derive(Clone, Copy)]
struct CapturedModel {
    model: ModelCapture,
    custom_skin: i32,
    features: ModelFeatures,
}
#[derive(Clone, Copy, Default)]
pub struct StaticEntity {
    pub record: EntityRecord,
    models: [Option<CapturedModel>; 4],
}
impl std::ops::Deref for StaticEntity {
    type Target = EntityRecord;
    fn deref(&self) -> &Self::Target {
        &self.record
    }
}

#[derive(Clone, Copy)]
enum SceneBinding {
    Mesh(ModelBinding),
    Inline {
        binding: InlineBinding,
        presentation: WorldPresentationId,
    },
}
enum SceneBindings<'a> {
    Mesh(ModelBindings<'a>),
    Inline(Option<(InlineBinding, WorldPresentationId)>),
}
impl Iterator for SceneBindings<'_> {
    type Item = SceneBinding;
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Mesh(bindings) => bindings.next().map(SceneBinding::Mesh),
            Self::Inline(binding) => {
                binding
                    .take()
                    .map(|(binding, presentation)| SceneBinding::Inline {
                        binding,
                        presentation,
                    })
            }
        }
    }
}

mod player_models;
pub(crate) use player_models::PlayerModelOptions;

/// Only derived numeric asset bindings live here. Accepted entity values and
/// history remain in the connection's existing snapshot ring.
pub struct ClientModelBindings {
    client: ClientId,
    range: IndexSpan,
    profile: Option<ModelProfile>,
    world: Option<(qa_compat::services::LoadedWorldBinding, WorldPresentationId)>,
    revision: u64,
    registrations: Box<[Option<ClientModel>]>,
    pending: qa_core::bits::BitSet,
    features: Box<[ModelFeatures]>,
    players: Option<player_models::PlayerBindings>,
    colors: Option<crate::model_assets::ModelColorId>,
}
impl ClientModelBindings {
    pub fn load(client: ClientId, layout: ConfigLayout) -> Self {
        let range = layout.resources[0];
        Self {
            client,
            range,
            profile: None,
            world: None,
            revision: 0,
            registrations: vec![None; range.count as usize].into_boxed_slice(),
            pending: qa_core::bits::BitSet::empty(range.count as usize),
            features: vec![ModelFeatures::default(); range.count as usize].into_boxed_slice(),
            players: player_models::PlayerBindings::load(layout),
            colors: None,
        }
    }

    /// Disconnect/rebinding clears all weak references into the old catalog.
    pub fn reset(&mut self) {
        self.revision = self.revision.wrapping_add(1);
        self.profile = None;
        self.world = None;
        self.registrations.fill(None);
        self.pending.clear();
        self.features.fill(ModelFeatures::default());
        if let Some(players) = &mut self.players {
            players.clear();
        }
        self.colors = None;
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn collision_world(&self) -> Option<(GeometryId, u32)> {
        self.world
            .and_then(|(world, _)| world.collision)
            .map(|geometry| (geometry, 0))
    }
    pub fn collision_model(&self, model: ModelRef) -> Option<(GeometryId, u32)> {
        if model.namespace != ResourceNamespace::Client(self.client) {
            return None;
        }
        match self.registrations.get(model.ordinal as usize)?.as_ref()? {
            ClientModel::Inline {
                binding, collision, ..
            } => collision.map(|geometry| (geometry, binding.index)),
            ClientModel::Mesh(_) => None,
        }
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
            return if self
                .players
                .as_ref()
                .is_some_and(|players| players.contains(index))
            {
                self.refresh_player_models(configs, models) == 0
            } else {
                true
            };
        }
        let ordinal = index - self.range.first as usize;
        self.revision = self.revision.wrapping_add(1);
        self.registrations[ordinal] = None;
        self.pending.remove(ordinal);
        self.features[ordinal] = ModelFeatures::default();
        let Some(row) = configs.config(index) else {
            return false;
        };
        if row.text.is_empty() || ordinal == 0 {
            return true;
        }
        let Ok(path) = std::str::from_utf8(row.text) else {
            return false;
        };
        if self.players.is_some() && path.starts_with('#') {
            return true;
        }
        let source = self
            .players
            .as_ref()
            .map_or(RuleSetId::Quake3, |players| players.source());
        self.features[ordinal] =
            qa_gameplay::rules::appearance::policy(source).model_features(row.text);
        if path.starts_with('*') {
            let Some((world, presentation)) = self.world else {
                return false;
            };
            let Some(binding) = inline_model_ordinal(row.text)
                .and_then(|index| models.resolve_inline(world.render, index))
            else {
                return false;
            };
            self.registrations[ordinal] = Some(ClientModel::Inline {
                binding,
                presentation: Some(presentation),
                collision: world.collision,
            });
            return true;
        }
        if let Some((binding, collision)) = models.loaded_collision_model(row.text) {
            let presentation = self
                .world
                .filter(|(world, _)| world.render == binding.world)
                .map(|(_, presentation)| presentation);
            self.registrations[ordinal] = Some(ClientModel::Inline {
                binding,
                presentation,
                collision,
            });
            return true;
        }
        self.pending.insert(ordinal);
        let Some(registration) = self
            .profile
            .and_then(|profile| models.registration(path, profile))
        else {
            return false;
        };
        self.registrations[ordinal] = Some(ClientModel::Mesh(registration));
        self.pending.remove(ordinal);
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
        self.colors = models.color_binding(profile);
        self.world = world.map(|(render, presentation)| {
            (
                qa_compat::services::LoadedWorldBinding {
                    render,
                    collision: models.collision_world(render),
                },
                presentation,
            )
        });
        self.refresh_all(configs, models)
    }

    /// A new accepted gamestate replaces rows without changing the loaded
    /// presentation profile or world binding.
    pub(crate) fn refresh_all(&mut self, configs: &ConfigStrings, models: &ModelCatalog) -> u32 {
        let missing = self.range.range().fold(0, |missing, index| {
            missing + u32::from(!self.config_changed(index, configs, models))
        });
        missing + self.refresh_player_models(configs, models) as u32
    }

    /// A cold catalog publication retries unresolved ordinals without changing
    /// accepted configstrings or walking every row during ordinary frames.
    pub(crate) fn admission_published(
        &mut self,
        configs: &ConfigStrings,
        models: &ModelCatalog,
    ) -> u32 {
        let mut rejected = 0;
        let mut ordinal = self.pending.next_set(0);
        while let Some(index) = ordinal {
            ordinal = self.pending.next_set(index + 1);
            if let Some(row) = self.range.get(index) {
                rejected += u32::from(!self.config_changed(row, configs, models));
            }
        }
        if let Some(players) = &mut self.players {
            players.dirty = true;
        }
        rejected + self.refresh_player_models(configs, models)
    }

    pub fn resolve<'a>(
        &self,
        reference: ModelRef,
        skin: i32,
        models: &'a ModelCatalog,
        policy: ModelMaterialPolicy,
    ) -> Option<ModelBindings<'a>> {
        match self.resolve_scene(reference, skin, models, policy)? {
            SceneBindings::Mesh(bindings) => Some(bindings),
            SceneBindings::Inline(_) => None,
        }
    }

    fn resolve_scene<'a>(
        &self,
        reference: ModelRef,
        skin: i32,
        models: &'a ModelCatalog,
        policy: ModelMaterialPolicy,
    ) -> Option<SceneBindings<'a>> {
        if reference.namespace != ResourceNamespace::Client(self.client) {
            return None;
        }
        let registration = self
            .registrations
            .get(reference.ordinal as usize)
            .copied()
            .flatten()?;
        self.resolve_registration(registration, skin, models, policy)
    }
    fn resolve_registration<'a>(
        &self,
        registration: ClientModel,
        skin: i32,
        models: &'a ModelCatalog,
        policy: ModelMaterialPolicy,
    ) -> Option<SceneBindings<'a>> {
        match registration {
            ClientModel::Mesh(registration) => models
                .bindings(registration, u32::try_from(skin).unwrap_or(0), policy)
                .map(SceneBindings::Mesh),
            ClientModel::Inline {
                binding,
                presentation,
                ..
            } => Some(SceneBindings::Inline(Some((binding, presentation?)))),
        }
    }
    fn registered_static(
        &self,
        handle: i32,
        presentation: Option<WorldPresentationId>,
        models: &ModelCatalog,
    ) -> Option<ClientModel> {
        if let Some(binding) = models.renderer_inline(handle) {
            let presentation = presentation.or_else(|| {
                self.world
                    .filter(|(world, _)| world.render == binding.world)
                    .map(|(_, presentation)| presentation)
            });
            return Some(ClientModel::Inline {
                binding,
                presentation,
                collision: models.collision_world(binding.world),
            });
        }
        Some(ClientModel::Mesh(
            models.renderer_registration(handle, self.profile?)?,
        ))
    }
    fn binding_features(
        &self,
        reference: ModelRef,
        source: RuleSetId,
        cell: usize,
    ) -> ModelFeatures {
        if reference.namespace != ResourceNamespace::Client(self.client) {
            return ModelFeatures::default();
        }
        match qa_gameplay::rules::appearance::model_input(
            source,
            reference.ordinal,
            Some(qa_core::primitives::ModelUse::Entity(cell)),
        ) {
            ModelInput::Resource(ordinal) | ModelInput::LinkedTranslucent(ordinal) => self
                .features
                .get(ordinal as usize)
                .copied()
                .unwrap_or_default(),
            ModelInput::Player => ModelFeatures::default(),
        }
    }
    pub fn capture_static(
        &self,
        record: EntityRecord,
        configs: &ConfigStrings,
        lookup: Option<&dyn qa_compat::services::ModelLookup>,
        world: Option<(WorldId, WorldPresentationId)>,
    ) -> StaticEntity {
        let models = std::array::from_fn(|slot| {
            let reference = record.appearance.models[slot]?;
            if reference.namespace != ResourceNamespace::Client(self.client) {
                return None;
            }
            let policy = qa_gameplay::rules::appearance::policy(record.appearance.source);
            let ordinal = match policy
                .model_cell(
                    &record.appearance,
                    slot,
                    policy.project(&record.appearance),
                    ModelFeatures::default(),
                )?
                .model
            {
                ModelInput::Resource(ordinal) | ModelInput::LinkedTranslucent(ordinal) => ordinal,
                ModelInput::Player => return self.player_model(record.appearance.skin, slot),
            };
            if let Some(binding) = self.registrations.get(ordinal as usize).copied().flatten() {
                return Some(CapturedModel {
                    model: ModelCapture::Bound(binding),
                    custom_skin: 0,
                    features: self
                        .features
                        .get(ordinal as usize)
                        .copied()
                        .unwrap_or_default(),
                });
            }
            let lookup = lookup?;
            let row = configs.config(self.range.get(ordinal as usize)?)?;
            if row.text.is_empty() {
                return None;
            }
            let (handle, presentation) = if row.text.starts_with(b"*") {
                let index = inline_model_ordinal(row.text)?;
                let (world, presentation) = self
                    .world
                    .map(|(world, presentation)| (world.render, presentation))
                    .or(world)?;
                (lookup.inline_handle(world, index)?, Some(presentation))
            } else {
                (lookup.file_handle(row.text)?, None)
            };
            Some(CapturedModel {
                model: ModelCapture::Registered {
                    handle,
                    presentation,
                },
                custom_skin: 0,
                features: policy.model_features(row.text),
            })
        });
        StaticEntity { record, models }
    }
    fn complete_capture(
        &self,
        capture: CapturedModel,
        models: &ModelCatalog,
    ) -> Option<CapturedModel> {
        let model = match capture.model {
            ModelCapture::Bound(model) => model,
            ModelCapture::Registered {
                handle,
                presentation,
            } => self.registered_static(handle, presentation, models)?,
        };
        if capture.custom_skin != 0 && !models.renderer_skin(capture.custom_skin, self.profile?) {
            return None;
        }
        Some(CapturedModel {
            model: ModelCapture::Bound(model),
            ..capture
        })
    }
    pub(crate) fn complete_static(&self, entity: &mut StaticEntity, models: &ModelCatalog) {
        for slot in &mut entity.models {
            *slot = slot.and_then(|capture| {
                self.complete_capture(capture, models).or_else(|| {
                    let profile = self.profile?;
                    let decoded_model = match capture.model {
                        ModelCapture::Bound(_) => true,
                        ModelCapture::Registered { handle, .. } => models.renderer_candidate(
                            qa_compat::services::RendererRegistration::Model,
                            handle,
                            profile,
                        ),
                    };
                    let decoded_skin = capture.custom_skin == 0
                        || models.renderer_candidate(
                            qa_compat::services::RendererRegistration::Skin,
                            capture.custom_skin,
                            profile,
                        );
                    // Keep the original identity while its decoded resources
                    // await backend publication, even if its alias is rewritten.
                    (decoded_model && decoded_skin).then_some(capture)
                })
            });
        }
    }
    fn resolve_static<'a>(
        &self,
        binding: CapturedModel,
        skin: i32,
        models: &'a ModelCatalog,
        mut policy: ModelMaterialPolicy,
    ) -> Option<SceneBindings<'a>> {
        if binding.custom_skin != 0 {
            policy.custom_skin = binding.custom_skin;
        }
        let binding = self.complete_capture(binding, models)?;
        let ModelCapture::Bound(registration) = binding.model else {
            return None;
        };
        self.resolve_registration(registration, skin, models, policy)
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

    pub(crate) fn row<'a>(frame: ReceivedFrame<'a>, number: u32) -> Option<(&'a [u32], bool)> {
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
    ) -> Option<PresentedEntity> {
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
        let (origin, angles) = if let (Some(rules), Some(published)) = (
            self.projection.trajectory_rules(),
            record.appearance.presentation,
        ) {
            use qa_gameplay::rules::entities::TrajectorySample::Value;
            let mut position = published.trajectories.position;
            position.curve =
                rules.position_curve(position.curve, number, self.appearance.smooth_clients());
            if rules.interpolates(position.curve, number)
                && let Some(previous) = previous
                && let Some(old) = previous.appearance.presentation
            {
                let mut old_position = old.trajectories.position;
                old_position.curve = rules.position_curve(
                    old_position.curve,
                    number,
                    self.appearance.smooth_clients(),
                );
                let old_time = self.previous?.time();
                let current_time = self.current?.time();
                let origin = rules
                    .sample(&old_position, old_time, Value)?
                    .lerp(rules.sample(&position, current_time, Value)?, fraction);
                let old_angles = rules.sample(&old.trajectories.angles, old_time, Value)?;
                let current_angles =
                    rules.sample(&published.trajectories.angles, current_time, Value)?;
                let angles = Vec3(std::array::from_fn(|axis| {
                    lerp_angle(old_angles.0[axis], current_angles.0[axis], fraction)
                }));
                (origin, angles)
            } else {
                let mut origin = rules.sample(&position, self.interpolation.time, Value)?;
                let angles = rules.sample(
                    &published.trajectories.angles,
                    self.interpolation.time,
                    Value,
                )?;
                if let Some(ground) = published
                    .ground
                    .filter(|ground| ground.namespace == ResourceNamespace::Client(self.client))
                    && let Some((words, _)) = Self::row(self.current?, ground.ordinal)
                    && let Some(mover) = self.decode(words)
                    && rules.is_mover(ground.ordinal, mover.appearance.native.entity_type)
                    && let Some(mover) = mover.appearance.presentation
                {
                    let before =
                        rules.sample(&mover.trajectories.position, self.current?.time(), Value)?;
                    let after = rules.sample(
                        &mover.trajectories.position,
                        self.interpolation.time,
                        Value,
                    )?;
                    // Original cgame evaluates angular deltas but applies only
                    // mover translation. Both views borrow accepted CLIENT rows.
                    origin = origin + (after - before);
                }
                (origin, angles)
            }
        } else {
            let origin = if capabilities.frame_lerp
                || (capabilities.beam_radius.is_some() && !capabilities.beam_interpolate)
            {
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
            (origin, angles)
        };
        let angles =
            self.appearance
                .body_angles(angles, record.velocity, qw_player, self.alias_roll);
        Some(PresentedEntity {
            record,
            previous,
            origin,
            angles,
            capabilities,
        })
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
        mut supplement: Option<(u32, EntityRecord)>,
        mut visit: impl FnMut(u32, Option<PresentedEntity>),
    ) {
        if let Some(frame) = self.current {
            visit_raw(
                frame,
                self.projection,
                self.client,
                |number, record, peer, _| {
                    if let Some((extra, value)) = supplement.filter(|(extra, _)| *extra < number) {
                        supplement = None;
                        visit(extra, self.present(extra, value, false, false));
                    }
                    let predicted = supplement
                        .filter(|(extra, _)| *extra == number)
                        .map(|(_, value)| value);
                    if predicted.is_some() {
                        supplement = None;
                    }
                    visit(
                        number,
                        record.and_then(|mut record| {
                            if record.appearance.loop_sound.is_none()
                                && let Some(predicted) = predicted
                            {
                                record.appearance.loop_sound = predicted.appearance.loop_sound;
                            }
                            self.present(number, record, peer, true)
                        }),
                    );
                },
            );
        }
        if let Some((number, record)) = supplement {
            visit(number, self.present(number, record, false, false));
        }
    }

    /// Numeric lookup in accepted CLIENT rows. The pose uses the same native
    /// discontinuity and interpolation path as the copied scene entities.
    pub fn get(&self, number: u32) -> Option<PresentedEntity> {
        if let Some((words, qw_player)) = Self::row(self.current?, number) {
            return self.present(number, self.decode(words)?, qw_player, true);
        }
        self.player_record()
            .filter(|(native, _)| *native == number)
            .and_then(|(_, record)| self.present(number, record, false, false))
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
    pub static_entities: &'a [StaticEntity],
    pub player: &'a PlayerState,
    /// Camera angle policy is resolved once by CLIENT; NQ retains its gun pose
    /// from before punch while Q2 follows the presented, kicked view.
    pub weapon_view_angles: Vec3,
    pub alias_roll: AliasRoll,
    /// Snapshot-domain time is independent of shader/refdef time.
    pub interpolation: Interpolation,
    pub gun_animation: Option<GunAnimation>,
    pub gun_previous: Option<ReceivedFrame<'a>>,
    /// Negotiated native view entity; common client/entity slots cannot supply it.
    pub view_entity: Option<u32>,
}

pub struct BeamInput<'a> {
    pub material: qa_render::MaterialId,
    pub random: &'a mut dyn FnMut() -> u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SceneCounts {
    pub entities: u32,
    pub models: u32,
    pub beams: u32,
    pub missing_models: u32,
    pub unsupported: u32,
    pub rejected: u32,
}
impl std::ops::AddAssign for SceneCounts {
    fn add_assign(&mut self, rhs: Self) {
        self.entities += rhs.entities;
        self.models += rhs.models;
        self.beams += rhs.beams;
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
        mut beam: Option<BeamInput<'_>>,
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
                None,
                Some(number),
                fraction,
                frame,
                assets,
                software,
                &mut counts,
                beam.as_mut(),
            );
        });
        for retained in self.static_entities {
            let record = retained.record;
            counts.entities += 1;
            self.append_presented(
                PresentedEntity {
                    record,
                    previous: None,
                    origin: record.position,
                    angles: record.angles,
                    capabilities: self.appearance.project(&record.appearance),
                },
                Some(&retained.models),
                None,
                1.0,
                frame,
                assets,
                software,
                &mut counts,
                beam.as_mut(),
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
        captured: Option<&[Option<CapturedModel>; 4]>,
        number: Option<u32>,
        fraction: f32,
        frame: &mut Frame,
        assets: &Assets,
        software: bool,
        counts: &mut SceneCounts,
        beam: Option<&mut BeamInput<'_>>,
    ) {
        let record = pose.record;
        let capabilities = pose.capabilities;
        counts.unsupported += u32::from(capabilities.unsupported != 0);
        if number.is_some_and(|number| self.view_entity == Some(number))
            || capabilities.viewer_model
        {
            return;
        }
        if let Some(radius) = capabilities.beam_radius {
            let Some(presentation) = beam else {
                counts.unsupported += 1;
                return;
            };
            let Some(colors) = self
                .bindings
                .colors
                .and_then(|binding| self.models.colors(binding))
            else {
                counts.missing_models += 1;
                return;
            };
            let shift = ((presentation.random)() & 3) * 8;
            let index = ((record.appearance.skin as u32 >> shift) & 255) as u8;
            let Some(palette) = assets.palette(colors.source) else {
                counts.missing_models += 1;
                return;
            };
            let color = palette.color(index).to_le_bytes();
            let end = if capabilities.beam_interpolate && !capabilities.frame_lerp {
                pose.previous.map_or(record.appearance.old_origin, |old| {
                    old.appearance
                        .old_origin
                        .lerp(record.appearance.old_origin, fraction)
                })
            } else {
                record.appearance.old_origin
            };
            let added = frame.add_entity(SceneEntity {
                model: SceneModel::Beam {
                    end,
                    radius,
                    world_space: true,
                },
                origin: pose.origin,
                alpha: Some(capabilities.alpha),
                palette_color: Some((colors.target, colors.indices[index as usize])),
                color: [
                    color[0],
                    color[1],
                    color[2],
                    (capabilities.alpha * 255.0) as u8,
                ],
                material: Some(presentation.material),
                ..Default::default()
            });
            counts.beams += u32::from(added);
            counts.rejected += u32::from(!added);
            return;
        }
        let previous = pose.previous;
        for (slot, reference) in record.appearance.models.iter().enumerate() {
            let Some(reference) = *reference else {
                continue;
            };
            let features = captured
                .and_then(|models| models[slot])
                .map(|model| model.features)
                .unwrap_or_else(|| {
                    self.bindings
                        .binding_features(reference, record.appearance.source, slot)
                });
            let Some(cell) =
                self.appearance
                    .model_cell(&record.appearance, slot, capabilities, features)
            else {
                continue;
            };
            let capabilities = cell.capabilities;
            let normalized = match cell.model {
                ModelInput::Resource(ordinal) | ModelInput::LinkedTranslucent(ordinal) => {
                    ModelRef::native(reference.namespace, ordinal)
                }
                ModelInput::Player => None,
            };
            let policy = ModelMaterialPolicy {
                translucent: capabilities.alpha != 1.0,
                fullbright: capabilities.fullbright,
                custom_skin: 0,
                frame: u32::try_from(record.appearance.frame).unwrap_or(0),
            };
            let resolved = if let Some(captured) = captured {
                captured[slot].and_then(|binding| {
                    self.bindings
                        .resolve_static(binding, cell.skin, self.models, policy)
                })
            } else {
                if cell.model == ModelInput::Player {
                    self.bindings
                        .player_model(record.appearance.skin, slot)
                        .and_then(|binding| {
                            self.bindings
                                .resolve_static(binding, cell.skin, self.models, policy)
                        })
                } else {
                    normalized.and_then(|reference| {
                        self.bindings
                            .resolve_scene(reference, cell.skin, self.models, policy)
                    })
                }
            };
            let Some(bindings) = resolved else {
                counts.missing_models += 1;
                continue;
            };
            for binding in bindings {
                let mut entity = SceneEntity {
                    origin: pose.origin,
                    non_normalized_axes: capabilities.scale != 1.0,
                    color: [255, 255, 255, (capabilities.alpha * 255.0) as u8],
                    alpha: Some(capabilities.alpha),
                    depth_hack: capabilities.depth_hack,
                    infrared_visible: capabilities.infrared_visible,
                    ..SceneEntity::default()
                };
                let angles = match binding {
                    SceneBinding::Inline {
                        binding,
                        presentation,
                    } => {
                        entity.material_opacity = capabilities
                            .inline_opacity
                            .and_then(qa_render::scene::MaterialOpacity::new);
                        // Native brush opacity covers the complete material;
                        // it must not also attenuate an entity-alpha stage.
                        if entity.material_opacity.is_some() {
                            entity.alpha = None;
                            entity.color[3] = 255;
                        }
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
                        let current_frame =
                            native_frame(record.appearance.frame, model.frame_count());
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
                        if capabilities.alias_motion && model.sprite.is_none() {
                            entity.previous_origin = Some(record.appearance.old_origin);
                        }
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
        let Some(bindings) = self.bindings.resolve(
            reference,
            weapon.skin,
            self.models,
            ModelMaterialPolicy::default(),
        ) else {
            counts.missing_models += 1;
            return;
        };
        let animation_policy = if self
            .player
            .view
            .flags
            .contains(qa_core::primitives::PlayerViewFlags::NO_WEAPON_LERP)
        {
            WeaponInterpolation::None
        } else {
            self.appearance.weapon_interpolation()
        };
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
                .filter(|_| {
                    player_event_interpolation(
                        self.current,
                        self.view_entity,
                        self.projection,
                        self.appearance,
                        self.client,
                    )
                })
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
        for binding in bindings {
            let Some(model) = assets.model(binding.model) else {
                counts.missing_models += 1;
                continue;
            };
            let angles = binding
                .pose
                .angles(self.weapon_view_angles + angles, software);
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
    }

    fn weapon(&self, frame: ReceivedFrame<'_>) -> Option<ViewWeapon> {
        self.player_projection.view_weapon(
            frame.player_words()?,
            ResourceNamespace::Client(self.client),
        )
    }
}

/// The selected native view entity can invalidate both camera and weapon
/// interpolation, independently of player origin and native flag changes.
pub(crate) fn player_event_interpolation(
    current: Option<ReceivedFrame<'_>>,
    number: Option<u32>,
    projection: &EntityProjection,
    appearance: AppearancePolicy,
    client: ClientId,
) -> bool {
    if !appearance.player_event_check() {
        return true;
    }
    let (Some(current), Some(number)) = (current, number) else {
        return true;
    };
    let Some((words, _)) = ClientEntities::row(current, number) else {
        return true;
    };
    let mut entity = EntityRecord::default();
    projection.apply(words, &mut entity, ResourceNamespace::Client(client))
        && !appearance.is_interpolation_event(&entity.appearance)
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

/// Decode the connection's accepted rows once through its shared projection.
/// Physics consumes these raw values; display interpolation remains separate.
pub(crate) fn visit_raw(
    frame: ReceivedFrame<'_>,
    projection: &EntityProjection,
    client: ClientId,
    mut visit: impl FnMut(u32, Option<EntityRecord>, bool, bool),
) {
    fn rows<const P: usize, const E: usize>(
        frame: SnapshotFrame<'_, P, E>,
        projection: &EntityProjection,
        client: ClientId,
        visit: &mut impl FnMut(u32, Option<EntityRecord>, bool, bool),
    ) {
        for row in frame.entities {
            let mut record = EntityRecord::default();
            let record = projection
                .apply(&row.words, &mut record, ResourceNamespace::Client(client))
                .then_some(record);
            visit(
                row.number,
                record,
                frame.qw_player(row.number),
                frame.qw_dead_player(row.number),
            );
        }
    }
    match frame {
        ReceivedFrame::NetQuake(frame) => rows(frame, projection, client, &mut visit),
        ReceivedFrame::QuakeWorld(frame) => rows(frame, projection, client, &mut visit),
        ReceivedFrame::Quake2(frame) => rows(frame, projection, client, &mut visit),
        ReceivedFrame::Quake2Repro(frame) => rows(frame, projection, client, &mut visit),
        ReceivedFrame::Quake3(frame) => rows(frame, projection, client, &mut visit),
    }
}
