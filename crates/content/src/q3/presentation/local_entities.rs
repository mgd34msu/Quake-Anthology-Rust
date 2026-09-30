//! Quake III presentation: local entities.
//!
//! Donor provenance: `src/content/q3/presentation/local-entities.ts`.

use qa_core::math::{
    add3, angles_to_axis, cross3, dot3, length3, normalize3, scale3, sub3, vec3, vec4, Axis, Bounds, Vec3, Vec4,
};
use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::world::{TraceContact, TraceSolidity};
use crate::q3::presentation::collision_host::TraceResult;
use crate::q3::presentation::effects::*;
use crate::q3::presentation::marks::*;
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::ref_entity::*;

// ---------------------------------------------------------------------------
// local-entities.ts
// ---------------------------------------------------------------------------

/// Maximum local entities.
pub const MAX_LOCAL_ENTITIES: usize = 512;

/// Puff-don't-scale flag.
pub const LE_PUFF_DONT_SCALE: i32 = 1;

/// Tumble flag.
pub const LE_TUMBLE: i32 = 2;

/// Sound 1 flag.
pub const LE_SOUND1: i32 = 4;

/// Sound 2 flag.
pub const LE_SOUND2: i32 = 8;

/// Local entity type (`leType`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LocalEntityType {
    /// Move, scale, fade.
    MoveScaleFade,
    /// Fall, scale, fade.
    FallScaleFade,
    /// Scale, fade.
    ScaleFade,
    /// Score plum.
    ScorePlum,
    /// Sprite explosion.
    SpriteExplosion,
    /// Fragment.
    Fragment,
    /// Kamikaze.
    Kamikaze,
    /// Invulnerability impact.
    InvulImpact,
    /// Invulnerability juiced.
    InvulJuiced,
    /// Fade RGB.
    FadeRgb,
    /// Explosion.
    Explosion,
    /// Mark.
    Mark,
    /// Show reference entity.
    ShowRefEntity,
}

/// Local entity mark type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalMarkType {
    /// None.
    None,
    /// Burn.
    Burn,
    /// Blood.
    Blood,
}

/// Local entity bounce sound type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalBounceSoundType {
    /// None.
    None,
    /// Blood.
    Blood,
    /// Brass.
    Brass,
}

/// Local entity record (`LocalEntity`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntity {
    /// Type.
    pub le_type: LocalEntityType,
    /// Reference entity.
    pub ref_entity: RefEntity,
    /// Flags.
    pub le_flags: i32,
    /// Start time.
    pub start_time: i32,
    /// End time.
    pub end_time: i32,
    /// Fade-in time.
    pub fade_in_time: i32,
    /// Life rate.
    pub life_rate: f32,
    /// Position trajectory.
    pub pos: Trajectory,
    /// Angle trajectory.
    pub angles: Trajectory,
    /// Bounce factor.
    pub bounce_factor: f32,
    /// Color.
    pub color: Vec4,
    /// Radius.
    pub radius: f32,
    /// Light.
    pub light: f32,
    /// Light color.
    pub light_color: Vec3,
    /// Mark type.
    pub le_mark_type: LocalMarkType,
    /// Bounce sound type.
    pub le_bounce_sound_type: LocalBounceSoundType,
}

/// Local entity media (`LocalEntityMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntityMedia {
    /// Blood trail shader.
    pub blood_trail_shader: Option<SceneShader>,
    /// Blood mark shader.
    pub blood_mark_shader: Option<SceneShader>,
    /// Burn mark shader.
    pub burn_mark_shader: Option<SceneShader>,
    /// Number shaders (0-9, minus).
    pub number_shaders: Vec<Option<SceneShader>>,
    /// Gib bounce sounds.
    pub gib_bounce_sounds: [Option<PresentSound>; 3],
}

/// Mission local entity media (`MissionLocalEntityMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionLocalEntityMedia {
    /// Base.
    pub base: LocalEntityMedia,
    /// Kamikaze shock wave.
    pub kamikaze_shock_wave: SceneModel,
    /// Kamikaze explode sound.
    pub kamikaze_explode_sound: Option<PresentSound>,
    /// Kamikaze implode sound.
    pub kamikaze_implode_sound: Option<PresentSound>,
}

/// Local entity host media variant.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalEntityHostMedia {
    /// Base.
    Base(LocalEntityMedia),
    /// Mission.
    Mission(MissionLocalEntityMedia),
}

/// Local entity host (`LocalEntityHost`).
pub struct LocalEntityHost {
    /// Prediction.
    pub prediction: Box<dyn PresentPrediction>,
    /// Collision.
    pub collision: Box<dyn PresentCollision>,
    /// Audio.
    pub audio: Box<dyn PresentAudio>,
    /// Client number.
    pub client_num: i32,
    /// Random.
    pub random: Box<dyn PresentRandom>,
    /// Marks.
    pub marks: Box<dyn PresentMarks>,
    /// Product.
    pub product: Product,
    /// Media.
    pub media: LocalEntityHostMedia,
}

/// Local entity frame (`LocalEntityFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LocalEntityFrame {
    /// Time.
    pub time: i32,
    /// Frame time.
    pub frame_time: i32,
    /// View origin.
    pub view_origin: Vec3,
}

/// Collected local entity scene (`LocalEntityScene`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntityScene {
    /// Entities.
    pub entities: Vec<RefEntity>,
    /// Dynamic lights.
    pub dynamic_lights: Vec<DynamicLight>,
}

/// Local entity scene sink (`LocalEntitySceneSink`).
pub trait LocalEntitySceneSink {
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: &RefEntity);
    /// Add a light.
    fn add_light(&mut self, light: &DynamicLight);
}

/// Local entity handle (slot index plus generation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalEntityHandle {
    /// Slot index.
    pub index: usize,
    /// Generation.
    pub generation: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct LocalSlot {
    prev: i32,
    next: i32,
    entity: Option<LocalEntity>,
    generation: u64,
    active: bool,
}

pub(crate) fn le_byte(value: f32) -> f32 {
    (qvm_float_to_int(value) & 255) as f32
}

pub(crate) fn le_remaining(entity: &LocalEntity, time: i32) -> f32 {
    (entity.end_time.wrapping_sub(time) as f32) * entity.life_rate
}

pub(crate) fn le_rgba(color: Vec4, scale: f32) -> Vec4 {
    vec4(
        le_byte(color.x * scale),
        le_byte(color.y * scale),
        le_byte(color.z * scale),
        le_byte(color.w * scale),
    )
}

pub(crate) fn le_scaled_axis(axis: &Axis, scale: f32) -> Axis {
    [scale3(axis[0], scale), scale3(axis[1], scale), scale3(axis[2], scale)]
}

/// Local entity pool (`LocalEntityPool`).
#[derive(Debug, Clone, PartialEq)]
pub struct LocalEntityPool {
    /// Product.
    pub product: Product,
    slots: Vec<LocalSlot>,
    head: i32,
    tail: i32,
    free_head: i32,
    count: usize,
    generation: u64,
}

impl LocalEntityPool {
    /// New pool.
    pub fn new(product: Product) -> Self {
        let mut pool = Self {
            product,
            slots: Vec::new(),
            head: -1,
            tail: -1,
            free_head: 0,
            count: 0,
            generation: 0,
        };
        pool.initialize();
        pool
    }

    /// Active count.
    #[must_use]
    pub fn active_count(&self) -> usize {
        self.count
    }

    /// Active entities newest-first.
    #[must_use]
    pub fn active_entities(&self) -> Vec<LocalEntity> {
        let mut entities = Vec::new();
        let mut index = self.head;
        while index != -1 {
            if let Some(entity) = &self.slots[index as usize].entity {
                entities.push(entity.clone());
            }
            index = self.slots[index as usize].next;
        }
        entities
    }

    /// Initialize all slots.
    pub fn initialize(&mut self) {
        self.head = -1;
        self.tail = -1;
        self.free_head = 0;
        self.count = 0;
        self.slots = (0..MAX_LOCAL_ENTITIES)
            .map(|index| LocalSlot {
                prev: -1,
                next: if index + 1 == MAX_LOCAL_ENTITIES {
                    -1
                } else {
                    index as i32 + 1
                },
                entity: None,
                generation: 0,
                active: false,
            })
            .collect();
    }

    /// Allocate a record (`allocate`).
    pub fn allocate(&mut self, le_type: LocalEntityType, ref_entity: RefEntity) -> PresentResult<LocalEntityHandle> {
        if self.product == Product::BaseQ3
            && matches!(
                le_type,
                LocalEntityType::Kamikaze
                    | LocalEntityType::InvulImpact
                    | LocalEntityType::InvulJuiced
                    | LocalEntityType::ShowRefEntity
            )
        {
            return Err(PresentError::state(format!(
                "{le_type:?} requires missionpack local entities"
            )));
        }
        match le_type {
            LocalEntityType::MoveScaleFade
            | LocalEntityType::FallScaleFade
            | LocalEntityType::ScaleFade
            | LocalEntityType::ScorePlum
            | LocalEntityType::SpriteExplosion => {
                if !matches!(ref_entity, RefEntity::Sprite(_)) {
                    return Err(PresentError::state(format!("{le_type:?} requires a sprite")));
                }
            }
            LocalEntityType::Fragment
            | LocalEntityType::Kamikaze
            | LocalEntityType::InvulImpact
            | LocalEntityType::InvulJuiced => {
                if !matches!(ref_entity, RefEntity::Model(_)) {
                    return Err(PresentError::state(format!("{le_type:?} requires a model")));
                }
            }
            LocalEntityType::FadeRgb | LocalEntityType::Explosion => {
                if matches!(ref_entity, RefEntity::Portal(_)) {
                    return Err(PresentError::state(format!("{le_type:?} requires a shaded entity")));
                }
            }
            LocalEntityType::Mark | LocalEntityType::ShowRefEntity => {}
        }
        if self.free_head == -1 {
            self.free_slot(self.tail)?;
        }
        let index = self.free_head as usize;
        let next_free = self.slots[index].next;
        self.free_head = next_free;
        self.generation += 1;
        let generation = self.generation;
        self.slots[index].entity = Some(LocalEntity {
            le_type,
            ref_entity,
            le_flags: 0,
            start_time: 0,
            end_time: 0,
            fade_in_time: 0,
            life_rate: 0.0,
            pos: Trajectory::default(),
            angles: Trajectory::default(),
            bounce_factor: 0.0,
            color: vec4(0.0, 0.0, 0.0, 0.0),
            radius: 0.0,
            light: 0.0,
            light_color: zero_vec3(),
            le_mark_type: LocalMarkType::None,
            le_bounce_sound_type: LocalBounceSoundType::None,
        });
        self.slots[index].active = true;
        self.slots[index].generation = generation;
        self.slots[index].prev = -1;
        self.slots[index].next = self.head;
        if self.head != -1 {
            self.slots[self.head as usize].prev = index as i32;
        } else {
            self.tail = index as i32;
        }
        self.head = index as i32;
        self.count += 1;
        Ok(LocalEntityHandle { index, generation })
    }

    /// Whether a handle is active (`isActive`).
    #[must_use]
    pub fn is_active(&self, handle: LocalEntityHandle) -> bool {
        self.slots
            .get(handle.index)
            .is_some_and(|slot| slot.active && slot.generation == handle.generation && slot.entity.is_some())
    }

    /// Read a record by slot, live or stale (`record`).
    pub fn read_record(&self, index: usize) -> PresentResult<&LocalEntity> {
        self.slots
            .get(index)
            .and_then(|slot| slot.entity.as_ref())
            .ok_or_else(|| PresentError::state("Local entity slot has no record"))
    }

    /// Read a live record by handle.
    pub fn get(&self, handle: LocalEntityHandle) -> Option<&LocalEntity> {
        self.is_active(handle)
            .then(|| self.slots[handle.index].entity.as_ref())
            .flatten()
    }

    /// Read a live record by handle, mutably.
    pub fn get_mut(&mut self, handle: LocalEntityHandle) -> Option<&mut LocalEntity> {
        if self.is_active(handle) {
            self.slots[handle.index].entity.as_mut()
        } else {
            None
        }
    }

    /// Free a record (`free`).
    pub fn free(&mut self, handle: LocalEntityHandle) -> PresentResult<()> {
        if !self.is_active(handle) {
            return Err(PresentError::state("CG_FreeLocalEntity: not active"));
        }
        self.free_slot(handle.index as i32)?;
        Ok(())
    }

    fn free_slot(&mut self, index: i32) -> PresentResult<()> {
        if index < 0 {
            return Err(PresentError::state("CG_FreeLocalEntity: not active"));
        }
        let slot = self
            .slots
            .get(index as usize)
            .ok_or_else(|| PresentError::range(format!("Invalid local entity slot {index}")))?;
        if !slot.active {
            return Err(PresentError::state("CG_FreeLocalEntity: not active"));
        }
        let (prev, next) = (slot.prev, slot.next);
        if prev == -1 {
            self.head = next;
        } else {
            self.slots[prev as usize].next = next;
        }
        if next == -1 {
            self.tail = prev;
        } else {
            self.slots[next as usize].prev = prev;
        }
        self.slots[index as usize].active = false;
        self.slots[index as usize].next = self.free_head;
        self.free_head = index;
        self.count -= 1;
        Ok(())
    }

    /// Visit oldest-first with the cached-prev semantics of the C array
    /// (`forEachOldestFirst`).
    pub fn for_each_oldest_first(&mut self, mut visit: impl FnMut(&mut Self, LocalEntityHandle)) {
        let mut index = self.tail;
        while index != -1 {
            let current = index as usize;
            let next = self.slots[current].prev;
            let handle = LocalEntityHandle {
                index: current,
                generation: self.slots[current].generation,
            };
            visit(self, handle);
            index = next;
        }
    }
}

/// Local entity processor (`LocalEntitySystem`).
pub struct LocalEntitySystem {
    /// Host.
    pub host: LocalEntityHost,
}

impl LocalEntitySystem {
    /// New system.
    pub fn new(host: LocalEntityHost) -> Self {
        Self { host }
    }

    fn check_pool(&self, pool: &LocalEntityPool) -> PresentResult<()> {
        if pool.product != self.host.product {
            return Err(PresentError::state("Local entity media product differs from its pool"));
        }
        Ok(())
    }

    /// Collect owned snapshots (`collectEntities`).
    pub fn collect_entities(
        &mut self,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        frame: &LocalEntityFrame,
    ) -> PresentResult<LocalEntityScene> {
        struct Collector {
            scene: LocalEntityScene,
        }
        impl LocalEntitySceneSink for Collector {
            fn add_ref_entity(&mut self, entity: &RefEntity) {
                self.scene.entities.push(copy_ref_entity(entity));
            }
            fn add_light(&mut self, light: &DynamicLight) {
                self.scene.dynamic_lights.push(*light);
            }
        }
        let mut collector = Collector {
            scene: LocalEntityScene {
                entities: Vec::new(),
                dynamic_lights: Vec::new(),
            },
        };
        self.add_entities(pool, effects, frame, &mut collector)?;
        Ok(collector.scene)
    }

    /// Add frame entities (`addEntities`).
    pub fn add_entities(
        &mut self,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        self.check_pool(pool)?;
        // Drive the loop here (rather than inside the pool) so callbacks can
        // borrow the pool, effects, host, and scene independently.
        let mut index = pool.tail;
        let mut first_error: Option<PresentError> = None;
        while index != -1 {
            let current = index as usize;
            // Cache the source prev pointer before each callback.
            let next = pool.slots[current].prev;
            let handle = LocalEntityHandle {
                index: current,
                generation: pool.slots[current].generation,
            };
            let entity = pool.read_record(current)?.clone();
            if frame.time >= entity.end_time {
                pool.free(handle)?;
            } else {
                let result = match entity.le_type {
                    LocalEntityType::Mark => Ok(()),
                    LocalEntityType::Fragment => Self::fragment(&mut self.host, pool, effects, handle, frame, scene),
                    LocalEntityType::MoveScaleFade | LocalEntityType::FallScaleFade | LocalEntityType::ScaleFade => {
                        Self::scale_fade(&mut self.host, pool, handle, frame, scene)
                    }
                    LocalEntityType::FadeRgb => {
                        if let Some(live) = pool.get_mut(handle) {
                            let fade = le_remaining(live, frame.time) * 255.0;
                            let rgba = le_rgba(live.color, fade);
                            set_shading_rgba(&mut live.ref_entity, rgba);
                            let entity = live.ref_entity.clone();
                            scene.add_ref_entity(&entity);
                        }
                        Ok(())
                    }
                    LocalEntityType::Explosion => {
                        if let Some(live) = pool.get(handle) {
                            let entity = live.ref_entity.clone();
                            scene.add_ref_entity(&entity);
                        }
                        if let Some(live) = pool.get(handle) {
                            let live = live.clone();
                            Self::explosion_light(&live, frame.time, scene);
                        }
                        Ok(())
                    }
                    LocalEntityType::SpriteExplosion => Self::sprite_explosion(pool, handle, frame, scene),
                    LocalEntityType::ScorePlum => Self::score_plum(&mut self.host, pool, handle, frame, scene),
                    LocalEntityType::Kamikaze => Self::kamikaze(&mut self.host, pool, handle, frame, scene),
                    LocalEntityType::InvulImpact | LocalEntityType::ShowRefEntity => {
                        if let Some(live) = pool.get(handle) {
                            let entity = live.ref_entity.clone();
                            scene.add_ref_entity(&entity);
                        }
                        Ok(())
                    }
                    LocalEntityType::InvulJuiced => Self::invulnerability_juiced(pool, effects, handle, frame, scene),
                };
                if let Err(error) = result {
                    first_error = Some(error);
                    break;
                }
            }
            index = next;
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        Ok(())
    }

    fn scale_fade(
        _host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let submission = {
            let Some(live) = pool.get_mut(handle) else {
                return Ok(());
            };
            let mut c = le_remaining(live, frame.time);
            if live.le_type == LocalEntityType::MoveScaleFade
                && live.fade_in_time > live.start_time
                && frame.time < live.fade_in_time
            {
                c = 1.0 - ((live.fade_in_time - frame.time) as f32) / ((live.fade_in_time - live.start_time) as f32);
            }
            let alpha = le_byte(255.0 * c * live.color.w);
            let radius_base = live.radius;
            let le_type = live.le_type;
            let le_flags = live.le_flags;
            let pos = live.pos;
            let mut submission = None;
            if let RefEntity::Sprite(re) = &mut live.ref_entity {
                re.shading.shader_rgba.w = alpha;
                if le_type != LocalEntityType::MoveScaleFade || le_flags & LE_PUFF_DONT_SCALE == 0 {
                    re.radius = radius_base * (1.0 - c)
                        + if le_type == LocalEntityType::FallScaleFade {
                            16.0
                        } else {
                            8.0
                        };
                }
                if le_type == LocalEntityType::MoveScaleFade {
                    re.origin = evaluate_trajectory(&pos, frame.time);
                } else if le_type == LocalEntityType::FallScaleFade {
                    re.origin.z = pos.base.z - (1.0 - c) * pos.delta.z;
                }
                if length3(sub3(re.origin, frame.view_origin)) < radius_base {
                    submission = Some(None);
                } else {
                    submission = Some(Some(RefEntity::Sprite(re.clone())));
                }
            }
            submission
        };
        match submission {
            Some(None) => {
                pool.free(handle)?;
            }
            Some(Some(entity)) => {
                scene.add_ref_entity(&entity);
            }
            None => {}
        }
        Ok(())
    }

    fn explosion_light(entity: &LocalEntity, time: i32, scene: &mut dyn LocalEntitySceneSink) {
        if entity.light == 0.0 {
            return;
        }
        let mut light = ((time - entity.start_time) as f32) / ((entity.end_time - entity.start_time) as f32);
        light = if light < 0.5 { 1.0 } else { 1.0 - (light - 0.5) * 2.0 };
        scene.add_light(&DynamicLight {
            origin: entity.ref_entity.origin(),
            radius: entity.light * light,
            color: entity.light_color,
            additive: false,
        });
    }

    fn sprite_explosion(
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let live = live.clone();
        let c = (((live.end_time - frame.time) as f32) / ((live.end_time - live.start_time) as f32)).min(1.0);
        if let RefEntity::Sprite(re) = &live.ref_entity {
            let mut re = re.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, le_byte(255.0 * c * 0.33));
            re.radius = 42.0 * (1.0 - c) + 30.0;
            scene.add_ref_entity(&RefEntity::Sprite(re));
        }
        Self::explosion_light(&live, frame.time, scene);
        Ok(())
    }

    fn fragment(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        if live.pos.type_ == TrajectoryType::Stationary {
            let t = live.end_time - frame.time;
            if t < 1000 {
                let Some(live) = pool.get_mut(handle) else {
                    return Ok(());
                };
                if let RefEntity::Model(re) = &mut live.ref_entity {
                    re.lighting_origin = re.origin;
                    re.shading.render_flags |= RF_LIGHTING_ORIGIN;
                    let origin = re.origin;
                    re.origin.z = origin.z - 16.0 * (1.0 - (t as f32) / 1000.0);
                    let entity = RefEntity::Model(re.clone());
                    scene.add_ref_entity(&entity);
                    re.origin = origin;
                }
            } else if let Some(live) = pool.get(handle) {
                let entity = live.ref_entity.clone();
                scene.add_ref_entity(&entity);
            }
            return Ok(());
        }
        let pos = live.pos;
        let origin = match &live.ref_entity {
            RefEntity::Model(re) => re.origin,
            other => other.origin(),
        };
        let new_origin = evaluate_trajectory(&pos, frame.time);
        let zero = zero_vec3();
        let trace = host
            .prediction
            .trace_mover(origin, new_origin, Bounds { min: zero, max: zero }, -1, 1);
        if trace.base.fraction == 1.0 {
            let Some(live) = pool.get_mut(handle) else {
                return Ok(());
            };
            let angles = live.angles;
            let tumble = live.le_flags & LE_TUMBLE != 0;
            let blood = live.le_bounce_sound_type == LocalBounceSoundType::Blood;
            if let RefEntity::Model(re) = &mut live.ref_entity {
                re.origin = new_origin;
                if tumble {
                    re.axis = angles_to_axis(evaluate_trajectory(&angles, frame.time));
                }
                let entity = RefEntity::Model(re.clone());
                scene.add_ref_entity(&entity);
            }
            if blood {
                Self::blood_trail(host, pool, effects, handle.index, frame)?;
            }
            return Ok(());
        }
        if host.collision.collision_contents(trace.base.end) & 0x80000000u32 as i32 != 0 {
            pool.free(handle)?;
            return Ok(());
        }
        let normal = match trace.base.contact {
            TraceContact::Plane { plane } => plane.normal,
            TraceContact::None => zero_vec3(),
        };
        if matches!(trace.base.contact, TraceContact::None) && trace.base.solidity != TraceSolidity::AllSolid {
            return Err(PresentError::state("Fragment impact has no trace plane"));
        }
        Self::bounce_mark(host, pool, handle, trace.base.end, normal);
        Self::bounce_sound(host, pool, handle, trace.base.end);
        Self::reflect_velocity(pool, handle, &trace.base, normal, frame);
        if let Some(live) = pool.get(handle) {
            let entity = live.ref_entity.clone();
            scene.add_ref_entity(&entity);
        }
        Ok(())
    }

    fn reflect_velocity(
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        trace: &TraceResult,
        normal: Vec3,
        frame: &LocalEntityFrame,
    ) {
        let Some(live) = pool.get_mut(handle) else { return };
        let hit_time =
            qvm_float_to_int(((frame.time - frame.frame_time) as f32) + (frame.frame_time as f32) * trace.fraction);
        let velocity = evaluate_trajectory_delta(&live.pos, hit_time);
        let dot = dot3(velocity, normal);
        let delta = scale3(add3(velocity, scale3(normal, -2.0 * dot)), live.bounce_factor);
        let stationary = trace.solidity == TraceSolidity::AllSolid
            || (normal.z > 0.0 && (delta.z < 40.0 || delta.z < (-(frame.frame_time as f32)) * delta.z));
        live.pos.base = trace.end;
        live.pos.time = frame.time;
        live.pos.delta = delta;
        if stationary {
            live.pos.type_ = TrajectoryType::Stationary;
        }
    }

    fn bounce_mark(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        origin: Vec3,
        normal: Vec3,
    ) {
        let (mark_type, blood_shader, burn_shader) = match pool.get_mut(handle) {
            Some(live) => {
                let media = match &host.media {
                    LocalEntityHostMedia::Base(media) => media,
                    LocalEntityHostMedia::Mission(media) => &media.base,
                };
                (
                    live.le_mark_type,
                    media.blood_mark_shader.clone(),
                    media.burn_mark_shader.clone(),
                )
            }
            None => return,
        };
        if mark_type != LocalMarkType::None {
            let blood = mark_type == LocalMarkType::Blood;
            let radius = (if blood { 16 } else { 8 } + (host.random.rand() & if blood { 31 } else { 15 })) as f32;
            let orientation = host.random.random() * 360.0;
            host.marks.impact_mark(&ImpactMarkRequest {
                shader: if blood { blood_shader } else { burn_shader },
                origin,
                direction: normal,
                orientation,
                color: vec4(1.0, 1.0, 1.0, 1.0),
                alpha_fade: true,
                radius,
                temporary: false,
            });
        }
        if let Some(live) = pool.get_mut(handle) {
            live.le_mark_type = LocalMarkType::None;
        }
    }

    fn bounce_sound(host: &mut LocalEntityHost, pool: &mut LocalEntityPool, handle: LocalEntityHandle, origin: Vec3) {
        let bounce_type = pool.get(handle).map(|live| live.le_bounce_sound_type);
        if bounce_type == Some(LocalBounceSoundType::Blood) && host.random.rand() & 1 != 0 {
            let value = host.random.rand() & 3;
            let media = match &host.media {
                LocalEntityHostMedia::Base(media) => media,
                LocalEntityHostMedia::Mission(media) => &media.base,
            };
            let sound = if value == 0 {
                media.gib_bounce_sounds[0].clone()
            } else if value == 1 {
                media.gib_bounce_sounds[1].clone()
            } else {
                media.gib_bounce_sounds[2].clone()
            };
            host.audio.start_sound(
                sound,
                &SoundOptions {
                    entity: ENTITYNUM_WORLD,
                    channel: 0,
                    origin: SoundOrigin::Fixed { position: origin },
                    volume: 127,
                },
            );
        }
        if let Some(live) = pool.get_mut(handle) {
            live.le_bounce_sound_type = LocalBounceSoundType::None;
        }
    }

    fn blood_trail(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        index: usize,
        frame: &LocalEntityFrame,
    ) -> PresentResult<()> {
        let step = 150;
        let start = step * ((frame.time - frame.frame_time + step) / step);
        let end = step * (frame.time / step);
        let mut time = start;
        let shader = match &host.media {
            LocalEntityHostMedia::Base(media) => media.blood_trail_shader.clone(),
            LocalEntityHostMedia::Mission(media) => media.base.blood_trail_shader.clone(),
        };
        let effect_frame = EffectFrame {
            time: frame.time,
            product: host.product,
            snap_client: None,
            predicted_client: 0,
        };
        while time <= end {
            let origin = evaluate_trajectory(&pool.read_record(index)?.pos, time);
            let handle = effects.smoke_puff(
                pool,
                &effect_frame,
                &SmokePuffOptions {
                    origin,
                    velocity: zero_vec3(),
                    radius: 20.0,
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                    duration: 2000,
                    start_time: time,
                    fade_in_time: 0,
                    flags: 0,
                    shader: shader.clone(),
                },
            )?;
            if let Some(blood) = pool.get_mut(handle) {
                blood.le_type = LocalEntityType::FallScaleFade;
                blood.pos.delta.z = 40.0;
            }
            time += step;
        }
        Ok(())
    }

    fn score_plum(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let live = live.clone();
        let c = le_remaining(&live, frame.time);
        let mut score = qvm_float_to_int(live.radius);
        let mut color = if score < 0 {
            vec4(255.0, 17.0, 17.0, 255.0)
        } else if score >= 50 {
            vec4(255.0, 0.0, 255.0, 255.0)
        } else if score >= 20 {
            vec4(0.0, 0.0, 255.0, 255.0)
        } else if score >= 10 {
            vec4(255.0, 255.0, 0.0, 255.0)
        } else if score >= 2 {
            vec4(0.0, 255.0, 0.0, 255.0)
        } else {
            vec4(255.0, 255.0, 255.0, 255.0)
        };
        if c < 0.25 {
            color.w = le_byte(255.0 * 4.0 * c);
        }
        let RefEntity::Sprite(mut re) = live.ref_entity.clone() else {
            return Ok(());
        };
        re.shading.shader_rgba = color;
        re.radius = 4.0;
        let mut origin = vec3(live.pos.base.x, live.pos.base.y, live.pos.base.z + (110.0 - c * 100.0));
        let direction = normalize3(cross3(sub3(frame.view_origin, origin), vec3(0.0, 0.0, 1.0)));
        let phase = c * 2.0 * std::f32::consts::PI;
        origin = add3(origin, scale3(direction, -10.0 + 20.0 * phase.sin()));
        if length3(sub3(origin, frame.view_origin)) < 20.0 {
            pool.free(handle)?;
            return Ok(());
        }
        let negative = score < 0;
        if negative {
            score = -score;
        }
        let mut digits = Vec::new();
        loop {
            digits.push((score % 10) as usize);
            score /= 10;
            if score == 0 {
                break;
            }
        }
        if negative {
            digits.push(10);
        }
        let media = match &host.media {
            LocalEntityHostMedia::Base(media) => media,
            LocalEntityHostMedia::Mission(media) => &media.base,
        };
        for (index, _) in digits.iter().enumerate() {
            re.origin = add3(
                origin,
                scale3(direction, ((digits.len() as f32) / 2.0 - index as f32) * 8.0),
            );
            let digit = digits[digits.len() - 1 - index];
            let shader = media
                .number_shaders
                .get(digit)
                .and_then(|shader| shader.clone())
                .ok_or_else(|| PresentError::state("Missing score digit shader"))?;
            re.shading.custom_shader = Some(shader);
            scene.add_ref_entity(&RefEntity::Sprite(re.clone()));
        }
        if let Some(live) = pool.get_mut(handle) {
            live.ref_entity = RefEntity::Sprite(re);
        }
        Ok(())
    }

    fn kamikaze(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        if host.product != Product::MissionPack {
            return Err(PresentError::state("Kamikaze requires missionpack media"));
        }
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let t = frame.time - live.start_time;
        let axis = angles_to_axis(zero_vec3());
        if t > 0 && t < 2000 {
            let sounded = pool.get(handle).is_some_and(|live| live.le_flags & LE_SOUND1 != 0);
            if !sounded {
                let sound = match &host.media {
                    LocalEntityHostMedia::Mission(media) => media.kamikaze_explode_sound.clone(),
                    LocalEntityHostMedia::Base(_) => None,
                };
                let client_num = host.client_num;
                host.audio.start_sound(
                    sound,
                    &SoundOptions {
                        entity: client_num,
                        channel: 0,
                        origin: SoundOrigin::Local,
                        volume: 127,
                    },
                );
                if let Some(live) = pool.get_mut(handle) {
                    live.le_flags |= LE_SOUND1;
                }
            }
            Self::shockwave(host, pool, handle, &axis, t, 0, 2000, 1500, 1320, scene)?;
        }
        if t > 250 && t < 2250 {
            let remaining = pool
                .get(handle)
                .map(|live| le_remaining(live, frame.time))
                .unwrap_or(0.0);
            let c = if t < 2000 {
                ((t - 250) as f32) / 1750.0
            } else {
                let sounded = pool.get(handle).is_some_and(|live| live.le_flags & LE_SOUND2 != 0);
                if !sounded {
                    let sound = match &host.media {
                        LocalEntityHostMedia::Mission(media) => media.kamikaze_implode_sound.clone(),
                        LocalEntityHostMedia::Base(_) => None,
                    };
                    let client_num = host.client_num;
                    host.audio.start_sound(
                        sound,
                        &SoundOptions {
                            entity: client_num,
                            channel: 0,
                            origin: SoundOrigin::Local,
                            volume: 127,
                        },
                    );
                    if let Some(live) = pool.get_mut(handle) {
                        live.le_flags |= LE_SOUND2;
                    }
                }
                ((2250 - t) as f32) / 250.0
            };
            let Some(live) = pool.get_mut(handle) else {
                return Ok(());
            };
            if let RefEntity::Model(re) = &mut live.ref_entity {
                re.shading.shader_rgba = le_rgba(live.color, remaining * 255.0);
                re.axis = le_scaled_axis(&axis, (c * 720.0) / 72.0);
                re.non_normalized_axes = true;
                let origin = re.origin;
                let entity = RefEntity::Model(re.clone());
                scene.add_ref_entity(&entity);
                scene.add_light(&DynamicLight {
                    origin,
                    radius: c * 1000.0,
                    color: vec3(1.0, 1.0, c),
                    additive: false,
                });
            }
        }
        if t > 2000 && t < 3000 {
            let needs_angles = pool.get(handle).is_some_and(|live| {
                live.angles.base.x == 0.0 && live.angles.base.y == 0.0 && live.angles.base.z == 0.0
            });
            if needs_angles {
                let angles = vec3(
                    host.random.random() * 360.0,
                    host.random.random() * 360.0,
                    host.random.random() * 360.0,
                );
                if let Some(live) = pool.get_mut(handle) {
                    live.angles.base = angles;
                }
            }
            let axis = pool
                .get(handle)
                .map(|live| angles_to_axis(live.angles.base))
                .unwrap_or(axis);
            Self::shockwave(host, pool, handle, &axis, t, 2000, 3000, 2500, 704, scene)?;
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn shockwave(
        host: &mut LocalEntityHost,
        pool: &mut LocalEntityPool,
        handle: LocalEntityHandle,
        axis: &Axis,
        time: i32,
        start: i32,
        end: i32,
        fade: i32,
        radius: i32,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let LocalEntityHostMedia::Mission(media) = &host.media else {
            return Err(PresentError::state("Shockwave requires missionpack media"));
        };
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let (shader_time, origin) = match &live.ref_entity {
            RefEntity::Model(re) => (re.shading.shader_time, re.origin),
            other => (0.0, other.origin()),
        };
        let mut re = create_model_entity(media.kamikaze_shock_wave.clone());
        re.shading.shader_time = shader_time;
        re.origin = origin;
        let c = ((time - start) as f32) / ((end - start) as f32);
        re.axis = le_scaled_axis(axis, (c * radius as f32) / 88.0);
        re.non_normalized_axes = true;
        let alpha = if time > fade {
            ((time - fade) as f32) / ((end - fade) as f32)
        } else {
            0.0
        };
        let channel = le_byte(255.0 - alpha * 255.0);
        re.shading.shader_rgba = vec4(channel, channel, channel, channel);
        scene.add_ref_entity(&RefEntity::Model(re));
        Ok(())
    }

    fn invulnerability_juiced(
        pool: &mut LocalEntityPool,
        effects: &mut ClientEffects,
        handle: LocalEntityHandle,
        frame: &LocalEntityFrame,
        scene: &mut dyn LocalEntitySceneSink,
    ) -> PresentResult<()> {
        let Some(live) = pool.get(handle) else { return Ok(()) };
        let t = frame.time - live.start_time;
        if t > 3000 {
            let xy = 1.0 + 0.3 * ((t - 3000) as f32) / 2000.0;
            let z = 0.7 + 0.3 * ((2000 - (t - 3000)) as f32) / 2000.0;
            if let Some(live) = pool.get_mut(handle) {
                if let RefEntity::Model(re) = &mut live.ref_entity {
                    re.axis[0].x = xy;
                    re.axis[1].y = xy;
                    re.axis[2].z = z;
                }
            }
        }
        if t > 5000 {
            let origin = pool
                .get(handle)
                .map(|live| live.ref_entity.origin())
                .unwrap_or(zero_vec3());
            if let Some(live) = pool.get_mut(handle) {
                live.end_time = 0;
            }
            let product = effects.product();
            let effect_frame = EffectFrame {
                time: frame.time,
                product,
                snap_client: None,
                predicted_client: 0,
            };
            effects.gib_player(pool, &effect_frame, origin)?;
        } else if let Some(live) = pool.get(handle) {
            let entity = live.ref_entity.clone();
            scene.add_ref_entity(&entity);
        }
        Ok(())
    }
}

pub(crate) fn set_shading_rgba(entity: &mut RefEntity, rgba: Vec4) {
    match entity {
        RefEntity::Model(re) => re.shading.shader_rgba = rgba,
        RefEntity::Sprite(re) => re.shading.shader_rgba = rgba,
        RefEntity::Beam(re) => re.shading.shader_rgba = rgba,
        RefEntity::RailCore(re) => re.shading.shader_rgba = rgba,
        RefEntity::RailRings(re) => re.shading.shader_rgba = rgba,
        RefEntity::Lightning(re) => re.shading.shader_rgba = rgba,
        RefEntity::Portal(_) => {}
    }
}
