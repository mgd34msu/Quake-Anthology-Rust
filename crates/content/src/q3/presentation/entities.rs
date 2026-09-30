//! Quake III presentation: entities.
//!
//! Donor provenance: `src/content/q3/presentation/entities.ts`.

use qa_core::math::{
    add3, angles_to_axis, cross3, dot3, length3, normalize3_or_zero, perpendicular_vector, rotate_point_around_vector,
    scale3, sub3, vec3, vec4, Axis, Vec3,
};
use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_scene::*;
use crate::q3::presentation::model_access::*;
use crate::q3::presentation::ref_entity::*;

// ---------------------------------------------------------------------------
// entities.ts
// ---------------------------------------------------------------------------

/// Solid brush-model marker.
pub const SOLID_BMODEL: i32 = 0xffffff;

/// Item channel.
pub const CHAN_ITEM: i32 = 4;

/// Body channel.
pub const CHAN_BODY: i32 = 5;

/// Missile trail kind (`MissileTrail`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissileTrail {
    /// Rocket.
    Rocket,
    /// Grenade.
    Grenade,
    /// Grapple.
    Grapple,
    /// Nail.
    Nail,
    /// Plasma.
    Plasma,
}

/// Packet weapon info (`PacketWeaponInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketWeaponInfo {
    /// Weapon model.
    pub weapon_model: SceneModel,
    /// Weapon midpoint.
    pub weapon_midpoint: Vec3,
    /// Barrel model.
    pub barrel_model: Option<SceneModel>,
    /// Missile model.
    pub missile_model: SceneModel,
    /// Missile render effects.
    pub missile_renderfx: i32,
    /// Missile sound.
    pub missile_sound: Option<PresentSound>,
    /// Missile dynamic light.
    pub missile_dlight: f32,
    /// Missile light color.
    pub missile_dlight_color: Vec3,
    /// Missile trail.
    pub missile_trail: Option<MissileTrail>,
    /// Trail radius.
    pub trail_radius: f32,
    /// Trail time.
    pub trail_time: i32,
}

/// Packet item visual (`PacketItemVisual`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketItemVisual {
    /// Models.
    pub models: [SceneModel; 2],
    /// Second model present.
    pub has_second: bool,
    /// Icon.
    pub icon: Option<SceneShader>,
}

/// Packet mission media (`PacketMissionMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketMissionMedia {
    /// Weapon hover sound.
    pub weapon_hover_sound: Option<PresentSound>,
    /// Blue prox mine.
    pub blue_prox_mine: SceneModel,
    /// Overload base model.
    pub overload_base_model: SceneModel,
    /// Overload energy model.
    pub overload_energy_model: SceneModel,
    /// Overload lights model.
    pub overload_lights_model: SceneModel,
    /// Overload target model.
    pub overload_target_model: SceneModel,
    /// Obelisk respawn sound.
    pub obelisk_respawn_sound: Option<PresentSound>,
    /// Harvester model.
    pub harvester_model: SceneModel,
    /// Harvester neutral model.
    pub harvester_neutral_model: SceneModel,
    /// Harvester red skin.
    pub harvester_red_skin: Option<SceneSkin>,
    /// Harvester blue skin.
    pub harvester_blue_skin: Option<SceneSkin>,
}

/// Packet entity media variant.
#[derive(Debug, Clone, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum PacketEntityMediaVariant {
    /// Base.
    Base,
    /// Mission.
    Mission(PacketMissionMedia),
}

/// Inline model entry.
#[derive(Debug, Clone, PartialEq)]
pub struct InlineModelEntry {
    /// Model.
    pub model: SceneModel,
    /// Midpoint.
    pub midpoint: Vec3,
}

/// Packet entity media (`PacketEntityMedia`).
#[derive(Debug, Clone, PartialEq)]
pub struct PacketEntityMedia {
    /// Game models.
    pub game_models: Vec<SceneModel>,
    /// Game sounds.
    pub game_sounds: Vec<Option<PresentSound>>,
    /// Inline models.
    pub inline_models: Vec<InlineModelEntry>,
    /// Items.
    pub items: Vec<PacketItemVisual>,
    /// Weapons.
    pub weapons: Vec<PacketWeaponInfo>,
    /// Plasma ball shader.
    pub plasma_ball_shader: Option<SceneShader>,
    /// Red flag base model.
    pub red_flag_base_model: SceneModel,
    /// Blue flag base model.
    pub blue_flag_base_model: SceneModel,
    /// Neutral flag base model.
    pub neutral_flag_base_model: SceneModel,
    /// Variant.
    pub variant: PacketEntityMediaVariant,
}

impl PacketEntityMedia {
    /// Product.
    #[must_use]
    pub fn product(&self) -> Product {
        match &self.variant {
            PacketEntityMediaVariant::Base => Product::BaseQ3,
            PacketEntityMediaVariant::Mission(_) => Product::MissionPack,
        }
    }
}

/// Packet entity imports (`PacketEntityImports`).
pub trait PacketEntityImports {
    /// Whether an entity body is hidden.
    fn body_hidden(&self, _entity: i32) -> bool {
        false
    }
    /// Pose an entity.
    fn pose_entity(&mut self, _entity: &mut ClientEntity) {}
    /// Add a reference entity.
    fn add_ref_entity(&mut self, entity: RefEntity);
    /// Add a light.
    fn add_light(&mut self, light: DynamicLight);
    /// Update a sound position.
    fn update_sound_position(&mut self, entity: i32, origin: Vec3);
    /// Add a loop sound.
    fn add_loop_sound(
        &mut self,
        entity: i32,
        origin: Vec3,
        velocity: Vec3,
        sound: Option<PresentSound>,
        real_loop: bool,
    );
    /// Start a sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity: i32, channel: i32, sound: Option<PresentSound>);
    /// Shared cgame `rand()`.
    fn random_integer(&mut self) -> i32;
    /// Present a player.
    fn present_player(&mut self, entity: &ClientEntity);
    /// Missile trail.
    fn missile_trail(&mut self, kind: MissileTrail, entity: &ClientEntity, weapon: &PacketWeaponInfo);
    /// Grapple trail.
    fn grapple_trail(&mut self, entity: &ClientEntity, weapon: &PacketWeaponInfo);
    /// Add an entity with powerups.
    fn add_entity_with_powerups(&mut self, entity: RefModelEntity, state: &EntityState, team: Team);
}

/// Packet entity options (`PacketEntityOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacketEntityOptions {
    /// Game type.
    pub game_type: GameType,
    /// Smooth clients.
    pub smooth_clients: bool,
    /// Simple items.
    pub simple_items: bool,
    /// Obelisk respawn delay.
    pub obelisk_respawn_delay: i32,
}

pub(crate) fn indexed<T: Clone>(values: &[T], index: i32, name: &str) -> PresentResult<T> {
    values
        .get(index as usize)
        .cloned()
        .ok_or_else(|| PresentError::range(format!("{name} index {index} is not registered")))
}

pub(crate) fn identity_axis_full() -> Axis {
    [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)]
}

pub(crate) fn scale_axis_full(axis: &Axis, scale: f32) -> Axis {
    [scale3(axis[0], scale), scale3(axis[1], scale), scale3(axis[2], scale)]
}

pub(crate) fn multiply_axis(left: &Axis, right: &Axis) -> Axis {
    let x = vec3(right[0].x, right[1].x, right[2].x);
    let y = vec3(right[0].y, right[1].y, right[2].y);
    let z = vec3(right[0].z, right[1].z, right[2].z);
    [
        vec3(dot3(left[0], x), dot3(left[0], y), dot3(left[0], z)),
        vec3(dot3(left[1], x), dot3(left[1], y), dot3(left[1], z)),
        vec3(dot3(left[2], x), dot3(left[2], y), dot3(left[2], z)),
    ]
}

pub(crate) fn tag_orientation(parent: &RefModelEntity, model: &SceneModel, name: &str) -> (Vec3, Axis) {
    match lerp_model_tag(model, name, parent.old_frame, parent.frame, 1.0 - parent.back_lerp) {
        None => (zero_vec3(), identity_axis_full()),
        Some(tag) => (tag.origin, tag.axes),
    }
}

pub(crate) fn tag_origin(parent: &RefModelEntity, tag: Vec3) -> Vec3 {
    let mut origin = add3(parent.origin, scale3(parent.axis[0], tag.x));
    origin = add3(origin, scale3(parent.axis[1], tag.y));
    add3(origin, scale3(parent.axis[2], tag.z))
}

/// Position an entity on a tag (`positionEntityOnTag`).
pub fn position_entity_on_tag(
    entity: &mut RefModelEntity,
    parent: &RefModelEntity,
    parent_model: &SceneModel,
    tag_name: &str,
) {
    let (origin, axis) = tag_orientation(parent, parent_model, tag_name);
    entity.origin = tag_origin(parent, origin);
    entity.axis = multiply_axis(&axis, &parent.axis);
    entity.back_lerp = parent.back_lerp;
}

/// Position a rotated entity on a tag (`positionRotatedEntityOnTag`).
pub fn position_rotated_entity_on_tag(
    entity: &mut RefModelEntity,
    parent: &RefModelEntity,
    parent_model: &SceneModel,
    tag_name: &str,
) {
    let (origin, axis) = tag_orientation(parent, parent_model, tag_name);
    entity.origin = tag_origin(parent, origin);
    entity.axis = multiply_axis(&multiply_axis(&entity.axis, &axis), &parent.axis);
}

/// Adjust a position for a mover (`adjustPositionForMover`).
#[must_use]
pub fn adjust_position_for_mover(
    state: &ClientGameState,
    input: Vec3,
    mover_num: i32,
    from_time: i32,
    to_time: i32,
) -> Vec3 {
    if mover_num <= 0 || mover_num >= ENTITYNUM_WORLD {
        return input;
    }
    let Some(mover) = state.entity_ref(mover_num as usize) else {
        return input;
    };
    let current = mover.current_state.clone();
    if current.e_type != EntityType::Mover as i32 {
        return input;
    }
    let old_origin = evaluate_trajectory(&current.pos, from_time);
    let _ = evaluate_trajectory(&current.apos, from_time);
    let origin = evaluate_trajectory(&current.pos, to_time);
    let _ = evaluate_trajectory(&current.apos, to_time);
    add3(input, sub3(origin, old_origin))
}

pub(crate) fn entity_lerp_angle(from: f32, to: f32, fraction: f32) -> f32 {
    let mut to = to;
    if to - from > 180.0 {
        to -= 360.0;
    }
    if to - from < -180.0 {
        to += 360.0;
    }
    from + fraction * (to - from)
}

pub(crate) fn entity_interpolate(from: Vec3, to: Vec3, fraction: f32) -> Vec3 {
    add3(from, scale3(sub3(to, from), fraction))
}

pub(crate) fn direction_axis(direction: Vec3, yaw: f32) -> Axis {
    let mut side = perpendicular_vector(direction);
    if yaw != 0.0 {
        side = rotate_point_around_vector(direction, side, f64::from(yaw));
    }
    [direction, side, cross3(direction, side)]
}

pub(crate) fn missile_direction(delta: Vec3) -> Vec3 {
    if length3(delta) == 0.0 {
        vec3(0.0, 0.0, 1.0)
    } else {
        normalize3_or_zero(delta)
    }
}

/// Packet entity presenter (`PacketEntityPresenter`).
pub struct PacketEntityPresenter {
    /// Media.
    pub media: PacketEntityMedia,
    /// Imports.
    pub imports: Box<dyn PacketEntityImports>,
    /// Item table.
    pub items: Box<dyn PresentItemTable>,
}

impl PacketEntityPresenter {
    /// New presenter.
    pub fn new(
        product: Product,
        media: PacketEntityMedia,
        imports: Box<dyn PacketEntityImports>,
        items: Box<dyn PresentItemTable>,
    ) -> PresentResult<Self> {
        if product != media.product() {
            return Err(PresentError::state(
                "packet entity media product differs from cgame state",
            ));
        }
        Ok(Self { media, imports, items })
    }

    fn body(&mut self, number: i32, reference: RefEntity) {
        if !self.imports.body_hidden(number) {
            self.imports.add_ref_entity(reference);
        }
    }

    /// Set an entity sound position (`setEntitySoundPosition`).
    pub fn set_entity_sound_position(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
    ) -> PresentResult<()> {
        let (number, origin, solid, modelindex) = match target {
            PresentEntityTarget::Predicted => {
                let entity = &state.predicted_player_entity;
                (
                    entity.current_state.number,
                    entity.lerp_origin,
                    entity.current_state.solid,
                    entity.current_state.modelindex,
                )
            }
            PresentEntityTarget::Indexed(index) => {
                let entity = state.entity_at(index);
                (
                    entity.current_state.number,
                    entity.lerp_origin,
                    entity.current_state.solid,
                    entity.current_state.modelindex,
                )
            }
        };
        let origin = if solid == SOLID_BMODEL {
            add3(
                origin,
                indexed(&self.media.inline_models, modelindex, "inline model")?.midpoint,
            )
        } else {
            origin
        };
        self.imports.update_sound_position(number, origin);
        Ok(())
    }

    /// Calculate lerp positions (`calculateLerpPositions`).
    pub fn calculate_lerp_positions(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        smooth_clients: bool,
    ) -> PresentResult<()> {
        let snap_time = state
            .snap
            .as_ref()
            .ok_or_else(|| PresentError::state("CG_CalcEntityLerpPositions: cg.snap == NULL"))?
            .server_time;
        let time = state.time;
        let frame_interpolation = state.frame_interpolation;
        // Borrow the entity once for flag/type updates.
        let (number, interpolate, pos_type) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            if !smooth_clients && entity.current_state.number < 64 {
                entity.current_state.pos.type_ = TrajectoryType::Interpolate;
                entity.next_state.pos.type_ = TrajectoryType::Interpolate;
            }
            (
                entity.current_state.number,
                entity.interpolate,
                entity.current_state.pos.type_,
            )
        };
        if interpolate
            && (pos_type == TrajectoryType::Interpolate || (pos_type == TrajectoryType::LinearStop && number < 64))
        {
            let next_time = state
                .next_snap
                .as_ref()
                .ok_or_else(|| PresentError::drop("CG_InterpoateEntityPosition: cg.nextSnap == NULL"))?
                .server_time;
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            let from_pos = evaluate_trajectory(&entity.current_state.pos, snap_time);
            let to_pos = evaluate_trajectory(&entity.next_state.pos, next_time);
            entity.lerp_origin = entity_interpolate(from_pos, to_pos, frame_interpolation);
            let a = evaluate_trajectory(&entity.current_state.apos, snap_time);
            let b = evaluate_trajectory(&entity.next_state.apos, next_time);
            entity.lerp_angles = vec3(
                entity_lerp_angle(a.x, b.x, frame_interpolation),
                entity_lerp_angle(a.y, b.y, frame_interpolation),
                entity_lerp_angle(a.z, b.z, frame_interpolation),
            );
            return Ok(());
        }
        let (pos, apos, ground) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.pos,
                entity.current_state.apos,
                entity.current_state.ground_entity_num,
            )
        };
        let mut lerp_origin = evaluate_trajectory(&pos, time);
        let lerp_angles = evaluate_trajectory(&apos, time);
        if !matches!(target, PresentEntityTarget::Predicted) {
            lerp_origin = adjust_position_for_mover(state, lerp_origin, ground, snap_time, time);
        }
        let entity = match target {
            PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
            PresentEntityTarget::Indexed(index) => state.entity_at(index),
        };
        entity.lerp_origin = lerp_origin;
        entity.lerp_angles = lerp_angles;
        Ok(())
    }

    fn effects(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        self.set_entity_sound_position(state, target)?;
        let (number, loop_sound, e_type, lerp_origin, constant_light) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.number,
                entity.current_state.loop_sound,
                entity.current_state.e_type,
                entity.lerp_origin,
                entity.current_state.constant_light,
            )
        };
        if loop_sound != 0 {
            let sound = indexed(&self.media.game_sounds, loop_sound, "sound")?;
            self.imports.add_loop_sound(
                number,
                lerp_origin,
                zero_vec3(),
                sound,
                e_type == EntityType::Speaker as i32,
            );
        }
        if constant_light != 0 {
            let light = constant_light as u32;
            self.imports.add_light(DynamicLight {
                origin: lerp_origin,
                radius: (((light >> 24) & 255) * 4) as f32,
                color: vec3(
                    (light & 255) as f32,
                    ((light >> 8) & 255) as f32,
                    ((light >> 16) & 255) as f32,
                ),
                additive: false,
            });
        }
        Ok(())
    }

    fn general(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (modelindex, frame, lerp_origin, lerp_angles, number) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.modelindex,
                entity.current_state.frame,
                entity.lerp_origin,
                entity.lerp_angles,
                entity.current_state.number,
            )
        };
        if modelindex == 0 {
            return Ok(());
        }
        let mut re = create_model_entity(indexed(&self.media.game_models, modelindex, "game model")?);
        re.frame = frame;
        re.old_frame = frame;
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        if state
            .snap
            .as_ref()
            .is_some_and(|snap| number == snap.player_state.client_num)
        {
            re.shading.render_flags |= RF_THIRD_PERSON;
        }
        re.axis = angles_to_axis(lerp_angles);
        self.body(number, RefEntity::Model(re));
        Ok(())
    }

    fn speaker(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (client_num, number, event_parm, frame, misc_time) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.client_num,
                entity.current_state.number,
                entity.current_state.event_parm,
                entity.current_state.frame,
                entity.misc_time,
            )
        };
        if client_num == 0 || state.time < misc_time {
            return Ok(());
        }
        let sound = indexed(&self.media.game_sounds, event_parm, "sound")?;
        self.imports.start_sound(None, number, CHAN_ITEM, sound);
        let random = ((self.imports.random_integer() & 0x7fff) as f32) / 0x7fff as f32;
        let crandom = 2.0 * (random - 0.5);
        let misc = qvm_float_to_int(
            state.time.wrapping_add(frame.wrapping_mul(100)) as f32 + client_num.wrapping_mul(100) as f32 * crandom,
        );
        let entity = match target {
            PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
            PresentEntityTarget::Indexed(index) => state.entity_at(index),
        };
        entity.misc_time = misc;
        Ok(())
    }

    fn item(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        let (modelindex, number, e_flags, misc_time) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.modelindex,
                entity.current_state.number,
                entity.current_state.e_flags,
                entity.misc_time,
            )
        };
        if modelindex as usize >= self.items.item_count(state.product) {
            return Err(PresentError::drop(format!("Bad item index {modelindex} on entity")));
        }
        if modelindex == 0 || e_flags & 0x80 != 0 {
            return Ok(());
        }
        let item = self
            .items
            .item_at(state.product, modelindex as usize)
            .ok_or_else(|| PresentError::drop(format!("Bad item index {modelindex} on entity")))?;
        let visual = indexed(&self.media.items, modelindex, "item visual")?;
        if options.simple_items && item.item_type != ItemType::Team {
            let lerp_origin = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
            };
            let mut re = create_sprite_entity();
            re.origin = lerp_origin;
            re.radius = 14.0;
            re.shading.custom_shader = visual.icon.clone();
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            self.body(number, RefEntity::Sprite(re));
            return Ok(());
        }
        let scale = 0.005 + number as f32 * 0.00001;
        let bob = 4.0 + (((state.time + 1000) as f32) * scale).cos() * 4.0;
        let fast = item.item_type == ItemType::Health;
        let (angles, axis) = if fast {
            (state.auto_angles_fast, state.auto_axis_fast)
        } else {
            (state.auto_angles, state.auto_axis)
        };
        let weapon = if item.item_type == ItemType::Weapon {
            Some(indexed(&self.media.weapons, item.tag, "weapon")?)
        } else {
            None
        };
        let mut re = create_model_entity(visual.models[0].clone());
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.lerp_origin = add3(entity.lerp_origin, vec3(0.0, 0.0, bob));
            entity.lerp_angles = angles;
            re.axis = axis;
            if let Some(weapon) = &weapon {
                let midpoint = weapon.weapon_midpoint;
                let offset = add3(
                    add3(scale3(re.axis[0], midpoint.x), scale3(re.axis[1], midpoint.y)),
                    scale3(re.axis[2], midpoint.z),
                );
                entity.lerp_origin = add3(sub3(entity.lerp_origin, offset), vec3(0.0, 0.0, 8.0));
            }
            re.origin = entity.lerp_origin;
            re.old_origin = entity.lerp_origin;
        }
        let msec = state.time.wrapping_sub(misc_time);
        let mut fraction = 1.0f32;
        if (0..1000).contains(&msec) {
            fraction = msec as f32 / 1000.0;
            re.axis = scale_axis_full(&re.axis, fraction);
            re.non_normalized_axes = true;
        }
        if item.item_type == ItemType::Weapon || item.item_type == ItemType::Armor {
            re.shading.render_flags |= RF_MINLIGHT;
        }
        if item.item_type == ItemType::Weapon {
            re.axis = scale_axis_full(&re.axis, 1.5);
            re.non_normalized_axes = true;
            if let PacketEntityMediaVariant::Mission(media) = &self.media.variant {
                let lerp_origin = match target {
                    PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
                };
                self.imports.add_loop_sound(
                    number,
                    lerp_origin,
                    zero_vec3(),
                    media.weapon_hover_sound.clone(),
                    false,
                );
            }
        }
        if self.media.product() == Product::MissionPack
            && item.item_type == ItemType::Holdable
            && item.tag == Holdable::Kamikaze as i32
        {
            re.axis = scale_axis_full(&re.axis, 2.0);
            re.non_normalized_axes = true;
        }
        self.body(number, RefEntity::Model(re.clone()));
        if self.media.product() == Product::MissionPack {
            if let Some(weapon) = &weapon {
                if let Some(barrel_model) = &weapon.barrel_model {
                    if !barrel_model.is_default() {
                        let mut barrel = create_model_entity(barrel_model.clone());
                        barrel.lighting_origin = re.lighting_origin;
                        barrel.shadow_plane = re.shadow_plane;
                        barrel.shading.render_flags = re.shading.render_flags;
                        position_rotated_entity_on_tag(&mut barrel, &re, &weapon.weapon_model, "tag_barrel");
                        barrel.axis = re.axis;
                        barrel.non_normalized_axes = re.non_normalized_axes;
                        self.body(number, RefEntity::Model(barrel));
                    }
                }
            }
        }
        if !options.simple_items
            && (item.item_type == ItemType::Health || item.item_type == ItemType::Powerup)
            && visual.has_second
            && !visual.models[1].is_default()
        {
            re.model = visual.models[1].clone();
            let mut yaw = 0.0;
            if item.item_type == ItemType::Powerup {
                re.origin = add3(re.origin, vec3(0.0, 0.0, 12.0));
                yaw = ((state.time & 1023) * 360) as f32 / -1024.0;
            }
            re.axis = angles_to_axis(vec3(0.0, yaw, 0.0));
            if fraction != 1.0 {
                re.axis = scale_axis_full(&re.axis, fraction);
                re.non_normalized_axes = true;
            }
            self.body(number, RefEntity::Model(re));
        }
        Ok(())
    }

    fn weapon_info(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
    ) -> PresentResult<PacketWeaponInfo> {
        // Source intentionally uses > rather than >= WP_NUM_WEAPONS.
        let count = if state.product == Product::MissionPack { 14 } else { 11 };
        let weapon = {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            if entity.current_state.weapon > count {
                entity.current_state.weapon = Weapon::None as i32;
            }
            entity.current_state.weapon
        };
        indexed(&self.media.weapons, weapon, "weapon")
    }

    fn missile(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let weapon = self.weapon_info(state, target)?;
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.lerp_angles = vec3(current.angles.x, current.angles.y, current.angles.z);
        }
        if let Some(kind) = weapon.missile_trail {
            let entity = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
                PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
            };
            self.imports.missile_trail(kind, &entity, &weapon);
        }
        if weapon.missile_dlight != 0.0 {
            self.imports.add_light(DynamicLight {
                origin: lerp_origin,
                radius: weapon.missile_dlight,
                color: weapon.missile_dlight_color,
                additive: false,
            });
        }
        if weapon.missile_sound.is_some() {
            let velocity = evaluate_trajectory_delta(&current.pos, state.time);
            self.imports.add_loop_sound(
                current.number,
                lerp_origin,
                velocity,
                weapon.missile_sound.clone(),
                false,
            );
        }
        if current.weapon == Weapon::Plasmagun as i32 {
            let mut re = create_sprite_entity();
            re.origin = lerp_origin;
            re.radius = 16.0;
            re.shading.custom_shader = self.media.plasma_ball_shader.clone();
            self.body(current.number, RefEntity::Sprite(re));
            return Ok(());
        }
        let mut re = create_model_entity(weapon.missile_model.clone());
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        re.skin_num = state.client_frame & 1;
        re.shading.render_flags = weapon.missile_renderfx | RF_NOSHADOW;
        if self.media.product() == Product::MissionPack
            && current.weapon == Weapon::ProxLauncher as i32
            && current.generic1 == Team::Blue as i32
        {
            if let PacketEntityMediaVariant::Mission(media) = &self.media.variant {
                re.model = media.blue_prox_mine.clone();
            }
        }
        let direction = missile_direction(current.pos.delta);
        if current.pos.type_ != TrajectoryType::Stationary {
            re.axis = direction_axis(direction, (state.time / 4) as f32);
        } else if state.product == Product::MissionPack && current.weapon == Weapon::ProxLauncher as i32 {
            let lerp_angles = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_angles,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_angles,
            };
            re.axis = angles_to_axis(lerp_angles);
        } else {
            re.axis = direction_axis(direction, current.time as f32);
        }
        if !self.imports.body_hidden(current.number) {
            self.imports.add_entity_with_powerups(re, &current, Team::Free);
        }
        Ok(())
    }

    fn grapple(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let weapon = self.weapon_info(state, target)?;
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            entity.lerp_angles = vec3(current.angles.x, current.angles.y, current.angles.z);
        }
        let entity = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
        };
        self.imports.grapple_trail(&entity, &weapon);
        let mut re = create_model_entity(weapon.missile_model.clone());
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        re.skin_num = state.client_frame & 1;
        re.shading.render_flags = weapon.missile_renderfx | RF_NOSHADOW;
        // CG_Grapple only fills axis[0]; the two cleared axes remain zero.
        re.axis = [missile_direction(current.pos.delta), re.axis[1], re.axis[2]];
        self.body(current.number, RefEntity::Model(re));
        Ok(())
    }

    fn mover(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) -> PresentResult<()> {
        let (solid, modelindex, modelindex2, number, lerp_origin, lerp_angles) = {
            let entity = match target {
                PresentEntityTarget::Predicted => &state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            (
                entity.current_state.solid,
                entity.current_state.modelindex,
                entity.current_state.modelindex2,
                entity.current_state.number,
                entity.lerp_origin,
                entity.lerp_angles,
            )
        };
        let model = if solid == SOLID_BMODEL {
            indexed(&self.media.inline_models, modelindex, "inline model")?.model
        } else {
            indexed(&self.media.game_models, modelindex, "game model")?
        };
        let mut re = create_model_entity(model);
        re.origin = lerp_origin;
        re.old_origin = lerp_origin;
        re.axis = angles_to_axis(lerp_angles);
        re.shading.render_flags = RF_NOSHADOW;
        re.skin_num = (state.time >> 6) & 1;
        self.body(number, RefEntity::Model(re.clone()));
        if modelindex2 != 0 {
            re.skin_num = 0;
            re.model = indexed(&self.media.game_models, modelindex2, "game model")?;
            self.body(number, RefEntity::Model(re));
        }
        Ok(())
    }

    /// Beam entity (`beam`).
    pub fn beam(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) {
        let (number, base, origin2) = match target {
            PresentEntityTarget::Predicted => {
                let entity = &state.predicted_player_entity;
                (
                    entity.current_state.number,
                    entity.current_state.pos.base,
                    entity.current_state.origin2,
                )
            }
            PresentEntityTarget::Indexed(index) => {
                let entity = state.entity_at(index);
                (
                    entity.current_state.number,
                    entity.current_state.pos.base,
                    entity.current_state.origin2,
                )
            }
        };
        let mut re = create_beam_entity();
        re.origin = base;
        re.old_origin = origin2;
        re.shading.render_flags = RF_NOSHADOW;
        re.axis = [vec3(1.0, 0.0, 0.0), vec3(0.0, 1.0, 0.0), vec3(0.0, 0.0, 1.0)];
        self.body(number, RefEntity::Beam(re));
    }

    fn portal(&mut self, state: &mut ClientGameState, target: PresentEntityTarget) {
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        let mut re = create_portal_entity();
        re.origin = lerp_origin;
        re.old_origin = current.origin2;
        let forward = byte_to_direction(current.event_parm);
        let side = sub3(zero_vec3(), perpendicular_vector(forward));
        re.axis = [forward, side, cross3(forward, side)];
        re.old_frame = current.powerups;
        re.frame = current.frame;
        re.skin_num = qvm_float_to_int(current.client_num as f32 / 256.0 * 360.0);
        self.body(current.number, RefEntity::Portal(re));
    }

    fn team(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        let current = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.clone(),
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.clone(),
        };
        let lerp_origin = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.lerp_origin,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).lerp_origin,
        };
        let mut re = create_model_entity(default_model());
        re.origin = lerp_origin;
        re.lighting_origin = lerp_origin;
        re.axis = angles_to_axis(current.angles);
        if options.game_type == GameType::Ctf
            || (state.product == Product::MissionPack && options.game_type == GameType::OneFlagCtf)
        {
            re.model = if current.modelindex == Team::Red as i32 {
                self.media.red_flag_base_model.clone()
            } else if current.modelindex == Team::Blue as i32 {
                self.media.blue_flag_base_model.clone()
            } else {
                self.media.neutral_flag_base_model.clone()
            };
            self.body(current.number, RefEntity::Model(re));
            return Ok(());
        }
        let PacketEntityMediaVariant::Mission(media) = self.media.variant.clone() else {
            return Ok(());
        };
        if options.game_type == GameType::Harvester {
            re.model = if current.modelindex == Team::Red as i32 || current.modelindex == Team::Blue as i32 {
                media.harvester_model.clone()
            } else {
                media.harvester_neutral_model.clone()
            };
            re.custom_skin = if current.modelindex == Team::Red as i32 {
                media.harvester_red_skin.clone()
            } else if current.modelindex == Team::Blue as i32 {
                media.harvester_blue_skin.clone()
            } else {
                None
            };
            self.body(current.number, RefEntity::Model(re));
            return Ok(());
        }
        if options.game_type != GameType::Obelisk {
            return Ok(());
        }
        re.model = media.overload_base_model.clone();
        self.body(current.number, RefEntity::Model(re.clone()));
        let health = qvm_float_to_int(current.modelindex2 as f32) & 255;
        if current.frame == 1 {
            re.shading.shader_rgba = vec4(255.0, health as f32, health as f32, 255.0);
            re.model = media.overload_energy_model.clone();
            self.body(current.number, RefEntity::Model(re.clone()));
        }
        if current.frame != 2 {
            {
                let entity = match target {
                    PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index),
                };
                entity.misc_time = 0;
                entity.muzzle_flash_time = 0;
            }
            re.shading.shader_rgba = vec4(255.0, health as f32, health as f32, 255.0);
            re.model = media.overload_lights_model.clone();
            self.body(current.number, RefEntity::Model(re.clone()));
            re.origin = add3(re.origin, vec3(0.0, 0.0, 56.0));
            re.model = media.overload_target_model.clone();
            self.body(current.number, RefEntity::Model(re));
            return Ok(());
        }
        let time = state.time;
        let misc_time = {
            let entity = match target {
                PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                PresentEntityTarget::Indexed(index) => state.entity_at(index),
            };
            if entity.misc_time == 0 {
                entity.misc_time = time;
            }
            entity.misc_time
        };
        let elapsed = state.time.wrapping_sub(misc_time);
        let threshold = options.obelisk_respawn_delay.wrapping_sub(5).wrapping_mul(1000);
        let scale = if elapsed > threshold {
            (((elapsed - threshold) as f32) / (threshold as f32)).min(1.0)
        } else {
            0.0
        };
        let color = (qvm_float_to_int(scale * 255.0) & 255) as f32;
        re.shading.shader_rgba = vec4(color, color, color, color);
        re.model = media.overload_lights_model.clone();
        self.body(current.number, RefEntity::Model(re.clone()));
        if elapsed > threshold {
            let muzzle = match target {
                PresentEntityTarget::Predicted => state.predicted_player_entity.muzzle_flash_time,
                PresentEntityTarget::Indexed(index) => state.entity_at(index).muzzle_flash_time,
            };
            if muzzle == 0 {
                self.imports
                    .start_sound(Some(lerp_origin), 1023, CHAN_BODY, media.obelisk_respawn_sound.clone());
                let entity = match target {
                    PresentEntityTarget::Predicted => &mut state.predicted_player_entity,
                    PresentEntityTarget::Indexed(index) => state.entity_at(index),
                };
                entity.muzzle_flash_time = 1;
            }
            let spin = 16.0 * (1.0 - scale).acos() * 180.0 / std::f32::consts::PI;
            re.axis = scale_axis_full(
                &angles_to_axis(vec3(current.angles.x, current.angles.y + spin, current.angles.z)),
                scale,
            );
            // Source leaves nonNormalizedAxes false even while scaling this target.
            re.shading.shader_rgba = vec4(255.0, 255.0, 255.0, 255.0);
            re.origin = add3(re.origin, vec3(0.0, 0.0, 56.0));
            re.model = media.overload_target_model.clone();
            self.body(current.number, RefEntity::Model(re));
        }
        Ok(())
    }

    /// Add an entity (`addEntity`).
    pub fn add_entity(
        &mut self,
        state: &mut ClientGameState,
        target: PresentEntityTarget,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        let type_ = match target {
            PresentEntityTarget::Predicted => state.predicted_player_entity.current_state.e_type,
            PresentEntityTarget::Indexed(index) => state.entity_at(index).current_state.e_type,
        };
        if type_ >= EntityType::Events as i32 {
            return Ok(());
        }
        self.calculate_lerp_positions(state, target, options.smooth_clients)?;
        match target {
            PresentEntityTarget::Predicted => {
                // Pose through a cloned entity to keep the borrow simple; the
                // donor pose hook only reads interpolated positions.
                let mut entity = state.predicted_player_entity.clone();
                self.imports.pose_entity(&mut entity);
                state.predicted_player_entity = entity;
            }
            PresentEntityTarget::Indexed(index) => {
                let mut entity = state.entity_at(index).clone();
                self.imports.pose_entity(&mut entity);
                *state.entity_at(index) = entity;
            }
        }
        self.effects(state, target)?;
        match EntityType::from_i32(type_) {
            Some(EntityType::Invisible) | Some(EntityType::PushTrigger) | Some(EntityType::TeleportTrigger) => Ok(()),
            Some(EntityType::General) => self.general(state, target),
            Some(EntityType::Player) => {
                let entity = match target {
                    PresentEntityTarget::Predicted => state.predicted_player_entity.clone(),
                    PresentEntityTarget::Indexed(index) => state.entity_at(index).clone(),
                };
                self.imports.present_player(&entity);
                Ok(())
            }
            Some(EntityType::Item) => self.item(state, target, options),
            Some(EntityType::Missile) => self.missile(state, target),
            Some(EntityType::Mover) => self.mover(state, target),
            Some(EntityType::Beam) => {
                self.beam(state, target);
                Ok(())
            }
            Some(EntityType::Portal) => {
                self.portal(state, target);
                Ok(())
            }
            Some(EntityType::Speaker) => self.speaker(state, target),
            Some(EntityType::Grapple) => self.grapple(state, target),
            Some(EntityType::Team) => self.team(state, target, options),
            Some(EntityType::Events) | None => Err(PresentError::drop(format!("Bad entity type: {type_}\n"))),
        }
    }

    /// Add packet entities (`addPacketEntities`).
    pub fn add_packet_entities(
        &mut self,
        state: &mut ClientGameState,
        options: &PacketEntityOptions,
    ) -> PresentResult<()> {
        if state.snap.is_none() {
            return Err(PresentError::state("CG_AddPacketEntities: cg.snap == NULL"));
        }
        let snap_time = state.snap.as_ref().map(|snap| snap.server_time).unwrap_or(0);
        let delta = state
            .next_snap
            .as_ref()
            .map(|next| next.server_time.wrapping_sub(snap_time))
            .unwrap_or(0);
        state.frame_interpolation = if delta == 0 {
            0.0
        } else {
            (state.time.wrapping_sub(snap_time) as f32) / (delta as f32)
        };
        state.auto_angles = vec3(0.0, ((state.time & 2047) * 360) as f32 / 2048.0, 0.0);
        state.auto_angles_fast = vec3(0.0, ((state.time & 1023) * 360) as f32 / 1024.0, 0.0);
        state.auto_axis = angles_to_axis(state.auto_angles);
        state.auto_axis_fast = angles_to_axis(state.auto_angles_fast);
        let predicted = state.predicted_player_state.clone();
        player_state_to_entity_state(&predicted, &mut state.predicted_player_entity.current_state);
        self.add_entity(state, PresentEntityTarget::Predicted, options)?;
        let client_num = state
            .snap
            .as_ref()
            .map(|snap| snap.player_state.client_num)
            .unwrap_or(0);
        self.calculate_lerp_positions(
            state,
            PresentEntityTarget::Indexed(client_num.max(0) as usize),
            options.smooth_clients,
        )?;
        let numbers: Vec<i32> = state
            .snap
            .as_ref()
            .map(|snap| snap.entities.iter().map(|entity| entity.number).collect())
            .unwrap_or_default();
        for number in numbers {
            self.add_entity(state, PresentEntityTarget::Indexed(number.max(0) as usize), options)?;
        }
        Ok(())
    }
}
