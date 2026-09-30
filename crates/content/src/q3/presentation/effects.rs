//! Quake III presentation: effects.
//!
//! Donor provenance: `src/content/q3/presentation/effects.ts`.

use qa_core::math::{
    add3, angles_to_axis, cross3, length3, normalize3, perpendicular_vector, rotate_point_around_vector, scale3, sub3,
    vec3, vec4, Axis, Vec3, Vec4,
};
use qa_core::numeric::{q_random, qvm_float_to_int};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::Product;
use crate::q3::base::shared::trajectory::{Trajectory, TrajectoryType};
use crate::q3::presentation::audio::PresentSound;
use crate::q3::presentation::local_entities::*;
use crate::q3::presentation::ref_entity::*;
use crate::q3::presentation::ref_entity::{PresentError, PresentResult};

// ---------------------------------------------------------------------------
// effects.ts
// ---------------------------------------------------------------------------

/// Mission effect media (`MissionEffectMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionEffectMedia {
    /// Lightning shader.
    pub lightning_shader: Option<SceneShader>,
    /// Kamikaze effect model.
    pub kamikaze_effect_model: SceneModel,
    /// Dish flash model.
    pub dish_flash_model: SceneModel,
    /// Rocket explosion shader.
    pub rocket_explosion_shader: Option<SceneShader>,
    /// Obelisk hit sounds.
    pub obelisk_hit_sounds: [Option<PresentSound>; 3],
    /// Invulnerability impact model.
    pub invulnerability_impact_model: SceneModel,
    /// Invulnerability impact sounds.
    pub invulnerability_impact_sounds: [Option<PresentSound>; 3],
    /// Invulnerability juiced model.
    pub invulnerability_juiced_model: SceneModel,
    /// Invulnerability juiced sound.
    pub invulnerability_juiced_sound: Option<PresentSound>,
}

/// Effect media variant.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum EffectMediaVariant {
    /// Base.
    Base {
        /// Teleport effect shader.
        teleport_effect_shader: Option<SceneShader>,
    },
    /// Mission.
    Mission(MissionEffectMedia),
}

/// Effect media (`EffectMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct EffectMedia {
    /// Water bubble shader.
    pub water_bubble_shader: Option<SceneShader>,
    /// Rage Pro smoke shader.
    pub smoke_puff_rage_pro_shader: Option<SceneShader>,
    /// Blood explosion shader.
    pub blood_explosion_shader: Option<SceneShader>,
    /// Teleport effect model.
    pub teleport_effect_model: SceneModel,
    /// Gib skull.
    pub gib_skull: SceneModel,
    /// Gib brain.
    pub gib_brain: SceneModel,
    /// Gib abdomen.
    pub gib_abdomen: SceneModel,
    /// Gib arm.
    pub gib_arm: SceneModel,
    /// Gib chest.
    pub gib_chest: SceneModel,
    /// Gib fist.
    pub gib_fist: SceneModel,
    /// Gib foot.
    pub gib_foot: SceneModel,
    /// Gib forearm.
    pub gib_forearm: SceneModel,
    /// Gib intestine.
    pub gib_intestine: SceneModel,
    /// Gib leg.
    pub gib_leg: SceneModel,
    /// Smoke 2.
    pub smoke2: SceneModel,
    /// Variant.
    pub variant: EffectMediaVariant,
}

impl EffectMedia {
    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        match &self.variant {
            EffectMediaVariant::Base { .. } => Product::Baseq3,
            EffectMediaVariant::Mission(_) => Product::Missionpack,
        }
    }
}

/// Effect options (`EffectOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectOptions {
    /// No projectile trail.
    pub no_projectile_trail: bool,
    /// Blood.
    pub blood: bool,
    /// Gibs.
    pub gibs: bool,
    /// Score plum.
    pub score_plum: bool,
    /// Hardware.
    pub hardware_rage_pro: bool,
}

/// Effect imports (`EffectImports`).
pub trait EffectImports {
    /// Cgame `rand()` (0..32767).
    fn random_integer(&mut self) -> i32;
    /// Start a sound.
    fn start_sound(&mut self, origin: Vec3, entity: i32, channel: i32, sound: Option<PresentSound>);
}

/// Smoke puff options (`SmokePuffOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct SmokePuffOptions {
    /// Origin.
    pub origin: Vec3,
    /// Velocity.
    pub velocity: Vec3,
    /// Radius.
    pub radius: f32,
    /// Color.
    pub color: Vec4,
    /// Duration.
    pub duration: i32,
    /// Start time.
    pub start_time: i32,
    /// Fade-in time.
    pub fade_in_time: i32,
    /// Flags.
    pub flags: i32,
    /// Shader.
    pub shader: Option<SceneShader>,
}

/// Explosion options (`ExplosionOptions`).
#[derive(Debug, Clone, PartialEq)]
pub struct ExplosionOptions {
    /// Origin.
    pub origin: Vec3,
    /// Direction.
    pub direction: Option<Vec3>,
    /// Model.
    pub model: SceneModel,
    /// Shader.
    pub shader: Option<SceneShader>,
    /// Duration.
    pub duration: i32,
    /// Sprite.
    pub sprite: bool,
}

pub(crate) fn effect_identity_axis() -> Axis {
    [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
}

pub(crate) fn effect_seconds(time: i32) -> f32 {
    (time as f32) / 1000.0
}

pub(crate) fn effect_life_rate(start: i32, end: i32) -> f32 {
    1.0 / (end.wrapping_sub(start) as f32)
}

pub(crate) fn effect_byte(value: f32) -> f32 {
    (qvm_float_to_int(value * 255.0) & 255) as f32
}

/// Client effects (`ClientEffects`).
pub struct ClientEffects {
    /// Media.
    pub media: EffectMedia,
    /// Options.
    pub options: EffectOptions,
    /// Imports.
    pub imports: Box<dyn EffectImports>,
    smoke_seed: i32,
    last_score_position: Vec3,
}

impl ClientEffects {
    /// New effects.
    pub fn new(
        product: Product,
        pool_product: Product,
        media: EffectMedia,
        options: EffectOptions,
        imports: Box<dyn EffectImports>,
    ) -> PresentResult<Self> {
        if product != media.product() {
            return Err(PresentError::state("Effect media product differs from cgame product"));
        }
        if product != pool_product {
            return Err(PresentError::state("Effect pool product differs from cgame product"));
        }
        Ok(Self {
            media,
            options,
            imports,
            smoke_seed: 0x92,
            last_score_position: zero_vec3(),
        })
    }

    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        self.media.product()
    }

    fn rand(&mut self) -> PresentResult<i32> {
        let value = self.imports.random_integer();
        if !(0..=32767).contains(&value) {
            return Err(PresentError::range("cgame rand() must return a 15-bit integer"));
        }
        Ok(value)
    }

    fn random(&mut self) -> PresentResult<f32> {
        Ok(((self.rand()? & 32767) as f32) / 32767.0)
    }

    fn crandom(&mut self) -> PresentResult<f32> {
        Ok(2.0 * (self.random()? - 0.5))
    }

    fn mission(&self) -> PresentResult<&MissionEffectMedia> {
        match &self.media.variant {
            EffectMediaVariant::Mission(media) => Ok(media),
            EffectMediaVariant::Base { .. } => Err(PresentError::state("Missionpack effect requested in baseq3")),
        }
    }

    /// Bubble trail (`bubbleTrail`).
    pub fn bubble_trail(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        start: Vec3,
        end: Vec3,
        spacing: f32,
    ) -> PresentResult<()> {
        if self.options.no_projectile_trail {
            return Ok(());
        }
        if !spacing.is_finite() || spacing.trunc() < 1.0 || spacing > 2_147_483_647.0 {
            return Err(PresentError::range(
                "Bubble spacing must have a positive integer divisor",
            ));
        }
        let difference = sub3(end, start);
        let length = length3(difference);
        let direction = normalize3(difference);
        let mut i = (self.rand()? % spacing.trunc() as i32) as f32;
        let mut position = add3(start, scale3(direction, i));
        let step = scale3(direction, spacing);
        while i < length {
            let mut re = create_sprite_entity();
            let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
            re.shading.shader_time = effect_seconds(frame.time);
            re.radius = 3.0;
            re.shading.custom_shader = self.media.water_bubble_shader.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            let random = self.random()?;
            let crandom = [self.crandom()?, self.crandom()?, self.crandom()?];
            if let Some(le) = pool.get_mut(handle) {
                le.le_flags = LE_PUFF_DONT_SCALE;
                le.start_time = frame.time;
                le.end_time = qvm_float_to_int(frame.time.wrapping_add(1000) as f32 + random * 250.0);
                le.life_rate = effect_life_rate(le.start_time, le.end_time);
                le.color = vec4(0.0, 0.0, 0.0, 1.0);
                le.pos = Trajectory {
                    trajectory_type: TrajectoryType::TrLinear,
                    time: frame.time,
                    duration: 0,
                    base: position,
                    delta: vec3(crandom[0] * 5.0, crandom[1] * 5.0, crandom[2] * 5.0 + 6.0),
                };
                le.ref_entity = RefEntity::Sprite(re);
            }
            position = add3(position, step);
            i = qvm_float_to_int(i + spacing) as f32;
        }
        Ok(())
    }

    /// Smoke puff (`smokePuff`).
    pub fn smoke_puff(
        &mut self,
        pool: &mut LocalEntityPool,
        _frame: &EffectFrame,
        options: &SmokePuffOptions,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_sprite_entity();
        let handle = pool.allocate(LocalEntityType::MoveScaleFade, RefEntity::Sprite(re.clone()))?;
        let rotation = q_random(self.smoke_seed);
        self.smoke_seed = rotation.seed;
        re.rotation = (rotation.value as f32) * 360.0;
        re.radius = options.radius;
        re.shading.shader_time = effect_seconds(options.start_time);
        re.origin = options.origin;
        re.shading.custom_shader = options.shader.clone();
        if self.options.hardware_rage_pro {
            re.shading.custom_shader = self.media.smoke_puff_rage_pro_shader.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
        } else {
            re.shading.shader_rgba = vec4(
                effect_byte(options.color.x),
                effect_byte(options.color.y),
                effect_byte(options.color.z),
                255.0,
            );
        }
        if let Some(le) = pool.get_mut(handle) {
            le.le_flags = options.flags;
            le.radius = options.radius;
            le.start_time = options.start_time;
            le.fade_in_time = options.fade_in_time;
            le.end_time = qvm_float_to_int(le.start_time as f32 + options.duration as f32);
            le.life_rate = effect_life_rate(
                if le.fade_in_time > le.start_time {
                    le.fade_in_time
                } else {
                    le.start_time
                },
                le.end_time,
            );
            le.color = options.color;
            le.pos = Trajectory {
                trajectory_type: TrajectoryType::TrLinear,
                time: le.start_time,
                duration: 0,
                base: options.origin,
                delta: options.velocity,
            };
            le.ref_entity = RefEntity::Sprite(re);
        }
        Ok(handle)
    }

    /// Spawn effect (`spawnEffect`).
    pub fn spawn_effect(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_model_entity(self.media.teleport_effect_model.clone());
        let handle = pool.allocate(LocalEntityType::FadeRgb, RefEntity::Model(re.clone()))?;
        let base = matches!(self.media.variant, EffectMediaVariant::Base { .. });
        re.shading.shader_time = effect_seconds(frame.time);
        re.axis = effect_identity_axis();
        re.origin = vec3(origin.x, origin.y, origin.z + if base { -24.0 } else { 16.0 });
        if let EffectMediaVariant::Base { teleport_effect_shader } = &self.media.variant {
            re.shading.custom_shader = teleport_effect_shader.clone();
        }
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(500);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Make an explosion (`makeExplosion`).
    ///
    /// The donor merges model and sprite records for sprite explosions; the
    /// extra model/old-origin fields are never read by sprite-explosion
    /// processing, so sprite explosions are plain sprite records here.
    pub fn make_explosion(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        options: &ExplosionOptions,
    ) -> PresentResult<LocalEntityHandle> {
        let duration = options.duration;
        if duration <= 0 {
            return Err(PresentError::range(format!("CG_MakeExplosion: msec = {duration}")));
        }
        if options.sprite && options.direction.is_none() {
            return Err(PresentError::state("Sprite explosion requires a direction vector"));
        }
        let offset = self.rand()? & 63;
        let handle = if options.sprite {
            let direction = options
                .direction
                .ok_or_else(|| PresentError::state("Sprite explosion requires a direction vector"))?;
            let mut re = create_sprite_entity();
            let handle = pool.allocate(LocalEntityType::SpriteExplosion, RefEntity::Sprite(re.clone()))?;
            re.rotation = (self.rand()? % 360) as f32;
            re.origin = add3(scale3(direction, 16.0), options.origin);
            if let Some(le) = pool.get_mut(handle) {
                le.ref_entity = RefEntity::Sprite(re);
            }
            handle
        } else {
            let mut re = create_model_entity(default_model());
            let handle = pool.allocate(LocalEntityType::Explosion, RefEntity::Model(re.clone()))?;
            if let Some(direction) = options.direction {
                let angle = self.rand()? % 360;
                let forward = direction;
                let perpendicular = perpendicular_vector(forward);
                let side = if angle == 0 {
                    perpendicular
                } else {
                    rotate_point_around_vector(forward, perpendicular, f64::from(angle))
                };
                re.axis = [forward, side, cross3(forward, side)];
            } else {
                re.axis = effect_identity_axis();
            }
            re.origin = options.origin;
            re.old_origin = options.origin;
            re.model = options.model.clone();
            if let Some(le) = pool.get_mut(handle) {
                le.ref_entity = RefEntity::Model(re);
            }
            handle
        };
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time.wrapping_sub(offset);
            le.end_time = le.start_time.wrapping_add(duration);
            let shader_time = effect_seconds(le.start_time);
            let rgba = le.ref_entity_shading_rgba();
            set_shading_rgba(&mut le.ref_entity, rgba);
            set_shading_time(&mut le.ref_entity, shader_time);
            set_custom_shader(&mut le.ref_entity, options.shader.clone());
            le.color = vec4(1.0, 1.0, 1.0, 0.0);
        }
        Ok(handle)
    }

    /// Bleed (`bleed`).
    pub fn bleed(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        entity_num: i32,
    ) -> PresentResult<()> {
        if !self.options.blood {
            return Ok(());
        }
        let Some(snap_client) = frame.snap_client else {
            return Err(PresentError::state("CG_Bleed requires a current snapshot"));
        };
        self.bleed_at(pool, frame, origin, entity_num == snap_client)?;
        Ok(())
    }

    /// Bleed at a point (`bleedAt`).
    pub fn bleed_at(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        hide_in_first_person: bool,
    ) -> PresentResult<Option<RefSpriteEntity>> {
        if !self.options.blood {
            return Ok(None);
        }
        let mut re = create_sprite_entity();
        let handle = pool.allocate(LocalEntityType::Explosion, RefEntity::Sprite(re.clone()))?;
        re.origin = origin;
        re.rotation = (self.rand()? % 360) as f32;
        re.radius = 24.0;
        re.shading.custom_shader = self.media.blood_explosion_shader.clone();
        if hide_in_first_person {
            re.shading.render_flags |= RF_THIRD_PERSON;
        }
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = le.start_time.wrapping_add(500);
            le.ref_entity = RefEntity::Sprite(re.clone());
        }
        Ok(Some(re))
    }

    /// Launch a gib (`launchGib`).
    pub fn launch_gib(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        velocity: Vec3,
        model: SceneModel,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_model_entity(model);
        let handle = pool.allocate(LocalEntityType::Fragment, RefEntity::Model(re.clone()))?;
        let random = self.random()?;
        re.origin = origin;
        re.axis = effect_identity_axis();
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = qvm_float_to_int(le.start_time.wrapping_add(5000) as f32 + random * 3000.0);
            le.pos = Trajectory {
                trajectory_type: TrajectoryType::TrGravity,
                time: frame.time,
                duration: 0,
                base: origin,
                delta: velocity,
            };
            le.bounce_factor = 0.6;
            le.le_bounce_sound_type = LocalBounceSoundType::Blood;
            le.le_mark_type = LocalMarkType::Blood;
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Gib a player (`gibPlayer`).
    pub fn gib_player(&mut self, pool: &mut LocalEntityPool, frame: &EffectFrame, origin: Vec3) -> PresentResult<()> {
        if !self.options.blood {
            return Ok(());
        }
        let first_velocity = vec3(
            self.crandom()? * 250.0,
            self.crandom()? * 250.0,
            250.0 + self.crandom()? * 250.0,
        );
        let head = if self.rand()? & 1 != 0 {
            self.media.gib_skull.clone()
        } else {
            self.media.gib_brain.clone()
        };
        self.launch_gib(pool, frame, origin, first_velocity, head)?;
        if !self.options.gibs {
            return Ok(());
        }
        let models = [
            self.media.gib_abdomen.clone(),
            self.media.gib_arm.clone(),
            self.media.gib_chest.clone(),
            self.media.gib_fist.clone(),
            self.media.gib_foot.clone(),
            self.media.gib_forearm.clone(),
            self.media.gib_intestine.clone(),
            self.media.gib_leg.clone(),
            self.media.gib_leg.clone(),
        ];
        for model in models {
            let next = vec3(
                self.crandom()? * 250.0,
                self.crandom()? * 250.0,
                250.0 + self.crandom()? * 250.0,
            );
            self.launch_gib(pool, frame, origin, next, model)?;
        }
        Ok(())
    }

    /// Launch debris (`launchExplode`).
    pub fn launch_explode(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        velocity: Vec3,
        model: SceneModel,
    ) -> PresentResult<LocalEntityHandle> {
        let mut re = create_model_entity(model);
        let handle = pool.allocate(LocalEntityType::Fragment, RefEntity::Model(re.clone()))?;
        let random = self.random()?;
        re.origin = origin;
        re.axis = effect_identity_axis();
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = qvm_float_to_int(le.start_time.wrapping_add(10000) as f32 + random * 6000.0);
            le.pos = Trajectory {
                trajectory_type: TrajectoryType::TrGravity,
                time: frame.time,
                duration: 0,
                base: origin,
                delta: velocity,
            };
            le.bounce_factor = 0.1;
            le.le_bounce_sound_type = LocalBounceSoundType::Brass;
            le.le_mark_type = LocalMarkType::None;
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Big explosion debris (`bigExplode`).
    pub fn big_explode(&mut self, pool: &mut LocalEntityPool, frame: &EffectFrame, origin: Vec3) -> PresentResult<()> {
        if !self.options.blood {
            return Ok(());
        }
        for scale in [1.0, 1.0, 1.5, 2.0, 2.5] {
            let velocity = vec3(
                self.crandom()? * 100.0 * scale,
                self.crandom()? * 100.0 * scale,
                150.0 + self.crandom()? * 100.0,
            );
            self.launch_explode(pool, frame, origin, velocity, self.media.smoke2.clone())?;
        }
        Ok(())
    }

    /// Score plum (`scorePlum`).
    pub fn score_plum(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        client: i32,
        origin: Vec3,
        score: i32,
    ) -> PresentResult<()> {
        if client != frame.predicted_client || !self.options.score_plum {
            return Ok(());
        }
        let re = create_sprite_entity();
        let handle = pool.allocate(LocalEntityType::ScorePlum, RefEntity::Sprite(re))?;
        let z = origin.z;
        let last_z = self.last_score_position.z;
        let base = vec3(
            origin.x,
            origin.y,
            if z >= last_z - 20.0 && z <= last_z + 20.0 {
                z - 20.0
            } else {
                z
            },
        );
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(4000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.radius = score as f32;
            le.pos.base = base;
            if let RefEntity::Sprite(re) = &mut le.ref_entity {
                re.radius = 16.0;
            }
        }
        self.last_score_position = origin;
        Ok(())
    }

    /// Lightning bolt beam (`lightningBoltBeam`).
    pub fn lightning_bolt_beam(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        start: Vec3,
        end: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let shader = self.mission()?.lightning_shader.clone();
        let mut re = create_lightning_entity();
        let handle = pool.allocate(LocalEntityType::ShowRefEntity, RefEntity::Lightning(re.clone()))?;
        re.origin = start;
        re.old_origin = end;
        re.shading.custom_shader = shader;
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(50);
            le.ref_entity = RefEntity::Lightning(re);
        }
        Ok(handle)
    }

    /// Kamikaze effect (`kamikazeEffect`).
    pub fn kamikaze_effect(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let model = self.mission()?.kamikaze_effect_model.clone();
        let mut re = create_model_entity(model);
        let handle = pool.allocate(LocalEntityType::Kamikaze, RefEntity::Model(re.clone()))?;
        re.shading.shader_time = effect_seconds(frame.time);
        re.origin = origin;
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(3000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        Ok(handle)
    }

    /// Obelisk explosion (`obeliskExplode`).
    pub fn obelisk_explode(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<()> {
        let media = self.mission()?.clone();
        let handle = self.make_explosion(
            pool,
            frame,
            &ExplosionOptions {
                origin: vec3(origin.x, origin.y, origin.z + 64.0),
                direction: Some(zero_vec3()),
                model: media.dish_flash_model,
                shader: media.rocket_explosion_shader,
                duration: 600,
                sprite: true,
            },
        )?;
        if let Some(le) = pool.get_mut(handle) {
            le.light = 300.0;
            le.light_color = vec3(1.0, 0.75, 0.0);
        }
        Ok(())
    }

    fn hit_sound(&mut self, sounds: &[Option<PresentSound>; 3]) -> PresentResult<Option<PresentSound>> {
        let choice = self.rand()? & 3;
        Ok(if choice < 2 {
            sounds[0].clone()
        } else if choice == 2 {
            sounds[1].clone()
        } else {
            sounds[2].clone()
        })
    }

    /// Obelisk pain (`obeliskPain`).
    pub fn obelisk_pain(&mut self, _pool: &mut LocalEntityPool, origin: Vec3) -> PresentResult<()> {
        let sounds = self.mission()?.obelisk_hit_sounds.clone();
        let sound = self.hit_sound(&sounds)?;
        self.imports.start_sound(origin, 1023, 5, sound);
        Ok(())
    }

    /// Invulnerability impact (`invulnerabilityImpact`).
    pub fn invulnerability_impact(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
        angles: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let media = self.mission()?.clone();
        let mut re = create_model_entity(media.invulnerability_impact_model);
        let handle = pool.allocate(LocalEntityType::InvulImpact, RefEntity::Model(re.clone()))?;
        re.shading.shader_time = effect_seconds(frame.time);
        re.origin = origin;
        re.axis = angles_to_axis(angles);
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(1000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        let sound = self.hit_sound(&media.invulnerability_impact_sounds)?;
        self.imports.start_sound(origin, 1023, 5, sound);
        Ok(handle)
    }

    /// Invulnerability juiced (`invulnerabilityJuiced`).
    pub fn invulnerability_juiced(
        &mut self,
        pool: &mut LocalEntityPool,
        frame: &EffectFrame,
        origin: Vec3,
    ) -> PresentResult<LocalEntityHandle> {
        let media = self.mission()?.clone();
        let mut re = create_model_entity(media.invulnerability_juiced_model);
        let handle = pool.allocate(LocalEntityType::InvulJuiced, RefEntity::Model(re.clone()))?;
        re.shading.shader_time = effect_seconds(frame.time);
        re.origin = origin;
        re.axis = angles_to_axis(zero_vec3());
        if let Some(le) = pool.get_mut(handle) {
            le.start_time = frame.time;
            le.end_time = frame.time.wrapping_add(10000);
            le.life_rate = effect_life_rate(le.start_time, le.end_time);
            le.color = vec4(1.0, 1.0, 1.0, 1.0);
            le.ref_entity = RefEntity::Model(re);
        }
        self.imports
            .start_sound(origin, 1023, 5, media.invulnerability_juiced_sound);
        Ok(handle)
    }
}

pub(crate) trait RefEntityShading {
    fn ref_entity_shading_rgba(&self) -> Vec4;
}

impl RefEntityShading for LocalEntity {
    fn ref_entity_shading_rgba(&self) -> Vec4 {
        match &self.ref_entity {
            RefEntity::Model(re) => re.shading.shader_rgba,
            RefEntity::Sprite(re) => re.shading.shader_rgba,
            RefEntity::Beam(re) => re.shading.shader_rgba,
            RefEntity::RailCore(re) => re.shading.shader_rgba,
            RefEntity::RailRings(re) => re.shading.shader_rgba,
            RefEntity::Lightning(re) => re.shading.shader_rgba,
            RefEntity::Portal(_) => vec4(0.0, 0.0, 0.0, 0.0),
        }
    }
}

pub(crate) fn set_shading_time(entity: &mut RefEntity, time: f32) {
    match entity {
        RefEntity::Model(re) => re.shading.shader_time = time,
        RefEntity::Sprite(re) => re.shading.shader_time = time,
        RefEntity::Beam(re) => re.shading.shader_time = time,
        RefEntity::RailCore(re) => re.shading.shader_time = time,
        RefEntity::RailRings(re) => re.shading.shader_time = time,
        RefEntity::Lightning(re) => re.shading.shader_time = time,
        RefEntity::Portal(_) => {}
    }
}

pub(crate) fn set_custom_shader(entity: &mut RefEntity, shader: Option<SceneShader>) {
    match entity {
        RefEntity::Model(re) => re.shading.custom_shader = shader,
        RefEntity::Sprite(re) => re.shading.custom_shader = shader,
        RefEntity::Beam(re) => re.shading.custom_shader = shader,
        RefEntity::RailCore(re) => re.shading.custom_shader = shader,
        RefEntity::RailRings(re) => re.shading.custom_shader = shader,
        RefEntity::Lightning(re) => re.shading.custom_shader = shader,
        RefEntity::Portal(_) => {}
    }
}

/// Effect frame: clock and snapshot identity for effect constructors.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EffectFrame {
    /// Time milliseconds.
    pub time: i32,
    /// Product.
    pub product: Product,
    /// Snapshot player client, when a snapshot is current.
    pub snap_client: Option<i32>,
    /// Predicted player client.
    pub predicted_client: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestEffects {
        next_rand: i32,
        sounds: Vec<(Vec3, i32, i32, Option<PresentSound>)>,
    }
    impl EffectImports for TestEffects {
        fn random_integer(&mut self) -> i32 {
            let value = self.next_rand;
            self.next_rand = (self.next_rand + 1) % 32768;
            value
        }
        fn start_sound(&mut self, origin: Vec3, entity: i32, channel: i32, sound: Option<PresentSound>) {
            self.sounds.push((origin, entity, channel, sound));
        }
    }

    fn test_media() -> EffectMedia {
        EffectMedia {
            water_bubble_shader: None,
            smoke_puff_rage_pro_shader: None,
            blood_explosion_shader: None,
            teleport_effect_model: default_model(),
            gib_skull: default_model(),
            gib_brain: default_model(),
            gib_abdomen: default_model(),
            gib_arm: default_model(),
            gib_chest: default_model(),
            gib_fist: default_model(),
            gib_foot: default_model(),
            gib_forearm: default_model(),
            gib_intestine: default_model(),
            gib_leg: default_model(),
            smoke2: default_model(),
            variant: EffectMediaVariant::Base {
                teleport_effect_shader: None,
            },
        }
    }

    fn test_options() -> EffectOptions {
        EffectOptions {
            no_projectile_trail: false,
            blood: true,
            gibs: true,
            score_plum: true,
            hardware_rage_pro: false,
        }
    }

    #[test]
    fn effects_smoke_and_explosion() {
        let mut pool = LocalEntityPool::new(Product::Baseq3);
        let mut effects = ClientEffects::new(
            Product::Baseq3,
            Product::Baseq3,
            test_media(),
            test_options(),
            Box::new(TestEffects {
                next_rand: 7,
                sounds: Vec::new(),
            }),
        )
        .unwrap();
        let frame = EffectFrame {
            time: 1000,
            product: Product::Baseq3,
            snap_client: Some(0),
            predicted_client: 0,
        };
        let handle = effects
            .smoke_puff(
                &mut pool,
                &frame,
                &SmokePuffOptions {
                    origin: zero_vec3(),
                    velocity: zero_vec3(),
                    radius: 10.0,
                    color: vec4(1.0, 1.0, 1.0, 1.0),
                    duration: 500,
                    start_time: 1000,
                    fade_in_time: 0,
                    flags: 0,
                    shader: None,
                },
            )
            .unwrap();
        assert_eq!(pool.get(handle).map(|le| le.end_time), Some(1500));
        let bad = effects.make_explosion(
            &mut pool,
            &frame,
            &ExplosionOptions {
                origin: zero_vec3(),
                direction: None,
                model: default_model(),
                shader: None,
                duration: 0,
                sprite: false,
            },
        );
        assert!(bad.is_err());
        effects
            .bubble_trail(&mut pool, &frame, zero_vec3(), vec3(0.0, 0.0, 64.0), 16.0)
            .unwrap();
        assert!(pool.active_count() >= 2);
        effects.gib_player(&mut pool, &frame, zero_vec3()).unwrap();
        assert!(pool.active_count() >= 12);
    }
}
