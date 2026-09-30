//! Quake III base/game: weapon.
//!
//! Donor provenance: `src/content/q3/base/game/weapon.ts`.

use qa_core::identity::ActorId;
use qa_core::math::add3;
use qa_core::math::angle_vectors;
use qa_core::math::dot3;
use qa_core::math::length3;
use qa_core::math::normalize3;
use qa_core::math::scale3;
use qa_core::math::sub3;
use qa_core::math::vec3;
use qa_core::math::vector_to_angles;
use qa_core::math::Bounds;
use qa_core::math::Vec3;
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::mover::*;
use crate::q3::base::game::state::*;

// ---------------------------------------------------------------------------
// weapon.ts: weapons (g_weapon.c, g_combat.c ray/invulnerability helpers)
// ---------------------------------------------------------------------------

/// Shot trace mask (`MASK_SHOT`).
pub(crate) const MASK_SHOT: i32 = 0x6000001;

/// Award flags (`AWARD_FLAGS`).
pub(crate) const AWARD_FLAGS: i32 = 0x8 | 0x40 | 0x800 | 0x8000 | 0x10000 | 0x20000;

/// Suicide means of death (`MOD_SUICIDE`).
pub(crate) const MOD_SUICIDE: i32 = 26;

/// Sphere intersections (`SphereIntersections`).
#[derive(Debug, Clone, PartialEq)]
pub enum SphereIntersections {
    /// None.
    None,
    /// One.
    One(Vec3),
    /// Two, in source order.
    Two(Vec3, Vec3),
}

/// Invulnerability impact (`InvulnerabilityImpact`).
#[derive(Debug, Clone, PartialEq)]
pub enum InvulnerabilityImpact {
    /// Miss.
    Miss,
    /// Hit.
    Hit {
        /// Impact point.
        impact_point: Vec3,
        /// Bounce direction.
        bounce_direction: Vec3,
    },
}

/// Weapon unlink hook (`WeaponHost.unlink`): returns the actor when a restore is required.
pub type WeaponUnlink = Rc<dyn Fn(&mut dyn Q3Driver, &ActorId) -> Option<ActorId>>;

/// Weapon relink hook (the donor restore closure).
pub type WeaponRelink = Rc<dyn Fn(&mut dyn Q3Driver, &ActorId)>;

pub(crate) fn client_of(pool: &dyn EntityPool, slot: usize) -> Result<usize, Q3GameError> {
    pool.entity(slot)
        .and_then(|entity| entity.client)
        .ok_or_else(|| failure("Weapon attack requires a client entity"))
}

/// Damage factor (`q3WeaponDamageFactor`). The persistant-powerup item tag is
/// resolved by the caller because entity/item links are slot-based here.
#[must_use]
pub fn q3_weapon_damage_factor(
    client: &GameClient,
    quad_factor: f32,
    persistant_powerup_tag: Option<i32>,
    product: Q3Product,
) -> f32 {
    let mut factor = if client.ps.powerups.get(Q3Powerup::Quad as usize) != 0 {
        quad_factor
    } else {
        1.0
    };
    if product == Q3Product::Missionpack && persistant_powerup_tag == Some(Q3Powerup::Doubler as i32) {
        factor *= 2.0;
    }
    factor
}

/// Ray/sphere intersections (`raySphereIntersections`); normalizes `direction` in place.
pub fn ray_sphere_intersections(origin: Vec3, radius: f32, point: Vec3, direction: &mut Vec3) -> SphereIntersections {
    let dir = normalize3(*direction);
    *direction = dir;
    let offset = sub3(point, origin);
    let b = 2.0 * dot3(dir, offset);
    let c = dot3(offset, offset) - radius * radius;
    let discriminant = b * b - 4.0 * c;
    if discriminant > 0.0 {
        let root = discriminant.sqrt();
        let first = (-b + root) / 2.0;
        let second = (-b - root) / 2.0;
        SphereIntersections::Two(add3(point, scale3(dir, first)), add3(point, scale3(dir, second)))
    } else if discriminant == 0.0 {
        SphereIntersections::One(add3(point, scale3(dir, -b / 2.0)))
    } else {
        SphereIntersections::None
    }
}

/// Invulnerability effect (`invulnerabilityEffect`).
pub fn invulnerability_effect(
    driver: &mut dyn Q3Driver,
    target: usize,
    direction: Vec3,
    point: Vec3,
) -> Result<InvulnerabilityImpact, Q3GameError> {
    if driver.pool().product() != Q3Product::Missionpack {
        return Err(failure("Invulnerability effects require missionpack"));
    }
    let Some(client) = driver.pool().entity(target).and_then(|entity| entity.client) else {
        return Ok(InvulnerabilityImpact::Miss);
    };
    let origin = driver
        .pool()
        .client(client)
        .map(|client| client.ps.origin)
        .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
    let mut backwards = vec3(-direction.x, -direction.y, -direction.z);
    let intersections = ray_sphere_intersections(origin, 42.0, point, &mut backwards);
    let impact_point = match &intersections {
        SphereIntersections::Two(first, _) | SphereIntersections::One(first) => *first,
        SphereIntersections::None => return Ok(InvulnerabilityImpact::Miss),
    };
    let _ = intersections;
    let impact = driver.pool().temp_entity(origin, Q3EntityEvent::InvulImpact);
    let offset = sub3(impact_point, origin);
    let angles = vector_to_angles(offset);
    let mut pitch = angles.x + 90.0;
    if pitch > 360.0 {
        pitch -= 360.0;
    }
    if let Some(impact) = driver.pool().entity_mut(impact) {
        impact.s.angles = vec3(pitch, angles.y, angles.z);
    }
    Ok(InvulnerabilityImpact::Hit {
        impact_point,
        bounce_direction: normalize3(offset),
    })
}

pub(crate) fn accuracy_subject(pool: &dyn EntityPool, slot: usize, actor: &ActorId) -> AccuracySubject {
    let entity = pool.entity(slot);
    let client = entity
        .and_then(|entity| entity.client)
        .and_then(|client| pool.client(client));
    AccuracySubject {
        actor: actor.clone(),
        damageable: entity.is_some_and(|entity| entity.takedamage),
        player: client.is_some(),
        health: client
            .map(|client| client.ps.health())
            .unwrap_or_else(|| entity.map(|entity| entity.health).unwrap_or(0)),
        team: client.map(|client| client.sess.session_team),
    }
}

/// Accuracy-hit test for native entities (`logAccuracyHit`).
pub fn log_accuracy_hit(game_type: i32, driver: &mut dyn Q3Driver, target: usize, attacker: usize) -> bool {
    let target_takedamage = driver.pool().entity(target).is_some_and(|entity| entity.takedamage);
    let target_client = driver.pool().entity(target).and_then(|entity| entity.client);
    let attacker_client = driver.pool().entity(attacker).and_then(|entity| entity.client);
    let target_health = target_client.and_then(|client| driver.pool().client(client).map(|client| client.ps.health()));
    if !target_takedamage || target == attacker || target_client.is_none() || attacker_client.is_none() {
        return false;
    }
    if target_health.is_some_and(|health| health <= 0) {
        return false;
    }
    let target_actor = driver.pool().entity(target).map(|entity| entity.actor.clone());
    let attacker_actor = driver.pool().entity(attacker).map(|entity| entity.actor.clone());
    let (Some(target_actor), Some(attacker_actor)) = (target_actor, attacker_actor) else {
        return false;
    };
    q3_accuracy_hit(
        game_type >= Q3GameType::Team as i32,
        &accuracy_subject(driver.pool(), target, &target_actor),
        &accuracy_subject(driver.pool(), attacker, &attacker_actor),
    )
}

/// Weapon hitscan host (the `bullet`/`contactHost` services).
pub struct WeaponHitscanHost {
    entity: usize,
    attacker: ActorId,
    firing_weapon: i32,
    method: i32,
    unlink: WeaponUnlink,
    relink: WeaponRelink,
}

impl WeaponHitscanHost {
    fn trace(&mut self, driver: &mut dyn Q3Driver, start: Vec3, end: Vec3, pass: Option<&ActorId>) -> Q3TraceResult {
        driver.spatial().trace_actor(&Q3TraceQuery {
            start,
            end,
            shape: Q3TraceShape::Point,
            pass_actor: pass.cloned(),
            mask: MASK_SHOT,
        })
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        let state = driver.combat().actor_combat_state(actor)?;
        let target = driver.participant(actor);
        let native = match &target {
            Participant::Entity(slot) => Some(*slot),
            Participant::SharedActor(_) => None,
        };
        let source_attacker = driver.native_slot(&self.attacker);
        let attacker_state = source_attacker.map_or(
            AccuracySubject {
                actor: self.attacker.clone(),
                damageable: false,
                player: false,
                health: 0,
                team: None,
            },
            |slot| accuracy_subject(driver.pool(), slot, &self.attacker),
        );
        let observed = native.map_or(
            AccuracySubject {
                actor: actor.clone(),
                damageable: state.can_take_damage,
                player: driver.actor_is_player(actor),
                health: state.health,
                team: state.team,
            },
            |slot| accuracy_subject(driver.pool(), slot, actor),
        );
        let team_game = driver.combat().game_type() >= Q3GameType::Team as i32;
        let time = driver.combat().time();
        let invulnerable = native
            .and_then(|slot| driver.pool().entity(slot))
            .and_then(|entity| entity.client)
            .and_then(|client| driver.pool().client(client))
            .is_some_and(|client| client.invulnerability_time > time);
        Some(BulletTarget {
            damageable: observed.damageable,
            player: observed.player,
            accuracy_eligible: q3_accuracy_hit(team_game, &observed, &attacker_state),
            invulnerable,
        })
    }

    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        let participant = driver.participant(target);
        let host = Participant::Entity(self.entity);
        let mut direction = direction;
        driver.combat().damage(
            &participant,
            Some(&host),
            Some(&host),
            Some(&mut direction),
            Some(point),
            amount,
            0,
            self.method,
        );
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> Result<InvulnerabilityImpact, Q3GameError> {
        let participant = driver.participant(target);
        let Participant::Entity(slot) = participant else {
            return Err(failure("Q3 invulnerability requires its actual client behavior"));
        };
        invulnerability_effect(driver, slot, direction, point)
    }
}

impl BulletHost for WeaponHitscanHost {
    fn trace_hit(
        &mut self,
        driver: &mut dyn Q3Driver,
        start: Vec3,
        end: Vec3,
        pass: Option<&ActorId>,
    ) -> Q3TraceResult {
        self.trace(driver, start, end, pass)
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        WeaponHitscanHost::hit_target(self, driver, actor)
    }

    fn emit_hit(&mut self, driver: &mut dyn Q3Driver, hit: &BulletHit) {
        let event = driver.pool().temp_entity(
            hit.point,
            if hit.flesh {
                Q3EntityEvent::BulletHitFlesh
            } else {
                Q3EntityEvent::BulletHitWall
            },
        );
        let flesh_target = if hit.flesh { hit.target.clone() } else { None };
        let (event_parm, other) = match flesh_target {
            Some(target) => {
                let participant = driver.participant(&target);
                match participant {
                    Participant::Entity(slot)
                        if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) =>
                    {
                        let number = driver.pool().entity(slot).map(|entity| entity.s.number).unwrap_or(0);
                        (
                            number,
                            driver
                                .pool()
                                .entity(self.entity)
                                .map(|entity| entity.s.number)
                                .unwrap_or(0),
                        )
                    }
                    _ => panic!("Admitted Q3 map player has no native client behavior record"),
                }
            }
            None => (
                direction_to_byte(Some(hit.normal)) as i32,
                driver
                    .pool()
                    .entity(self.entity)
                    .map(|entity| entity.s.number)
                    .unwrap_or(0),
            ),
        };
        if let Some(event) = driver.pool().entity_mut(event) {
            event.s.event_parm = event_parm;
            event.s.other_entity_num = other;
        }
    }

    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        WeaponHitscanHost::apply_damage(self, driver, target, direction, point, amount);
    }

    fn credit_accuracy(&mut self, driver: &mut dyn Q3Driver) {
        if let Ok(client) = client_of(driver.pool(), self.entity) {
            if let Some(client) = driver.pool().client_mut(client) {
                client.accuracy_hits = client.accuracy_hits.wrapping_add(1);
            }
        }
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        WeaponHitscanHost::invulnerability_impact(self, driver, target, direction, point)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

impl ContactHost for WeaponHitscanHost {
    fn trace_hit(
        &mut self,
        driver: &mut dyn Q3Driver,
        start: Vec3,
        end: Vec3,
        pass: Option<&ActorId>,
    ) -> Q3TraceResult {
        self.trace(driver, start, end, pass)
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        WeaponHitscanHost::hit_target(self, driver, actor)
    }

    fn emit_contact(&mut self, driver: &mut dyn Q3Driver, event: &ContactEvent) {
        match event {
            ContactEvent::GauntletQuad => {
                driver.pool().add_event(self.entity, Q3EntityEvent::PowerupQuad, 0);
            }
            ContactEvent::LightningReflection { start, end } => {
                let event = driver.pool().temp_entity(*start, Q3EntityEvent::Lightningbolt);
                if let Some(event) = driver.pool().entity_mut(event) {
                    event.s.origin2 = *end;
                }
            }
            ContactEvent::Miss { point, normal } => {
                let event = driver.pool().temp_entity(*point, Q3EntityEvent::MissileMiss);
                if let Some(event) = driver.pool().entity_mut(event) {
                    event.s.event_parm = direction_to_byte(Some(*normal)) as i32;
                }
            }
            ContactEvent::Hit { point, normal, target } => {
                let participant = driver.participant(target);
                let slot = match participant {
                    Participant::Entity(slot)
                        if driver.pool().entity(slot).is_some_and(|entity| entity.client.is_some()) =>
                    {
                        slot
                    }
                    _ => panic!("Admitted Q3 map player has no native client behavior record"),
                };
                let number = driver.pool().entity(slot).map(|entity| entity.s.number).unwrap_or(0);
                let event = driver.pool().temp_entity(*point, Q3EntityEvent::MissileHit);
                if let Some(event) = driver.pool().entity_mut(event) {
                    event.s.other_entity_num = number;
                    event.s.event_parm = direction_to_byte(Some(*normal)) as i32;
                    event.s.weapon = self.firing_weapon;
                }
            }
        }
    }

    fn apply_damage(&mut self, driver: &mut dyn Q3Driver, target: &ActorId, direction: Vec3, point: Vec3, amount: i32) {
        WeaponHitscanHost::apply_damage(self, driver, target, direction, point, amount);
    }

    fn credit_accuracy(&mut self, driver: &mut dyn Q3Driver) {
        if driver.combat().actor_combat_state(&self.attacker).is_some() {
            let entity = self.entity;
            if let Ok(client) = client_of(driver.pool(), entity) {
                if let Some(client) = driver.pool().client_mut(client) {
                    client.accuracy_hits = client.accuracy_hits.wrapping_add(1);
                }
            }
        }
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        WeaponHitscanHost::invulnerability_impact(self, driver, target, direction, point)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

impl ShotgunHost for WeaponHitscanHost {
    fn begin_shotgun(&mut self, driver: &mut dyn Q3Driver, muzzle: Vec3, direction: Vec3) -> usize {
        let event = driver.pool().temp_entity(muzzle, Q3EntityEvent::Shotgun);
        if let Some(event) = driver.pool().entity_mut(event) {
            event.s.origin2 = direction;
        }
        event
    }

    fn emit_shotgun_seed(&mut self, driver: &mut dyn Q3Driver, event_slot: usize, seed: i32) {
        let number = driver
            .pool()
            .entity(self.entity)
            .map(|entity| entity.s.number)
            .unwrap_or(0);
        if let Some(event) = driver.pool().entity_mut(event_slot) {
            event.s.event_parm = seed;
            event.s.other_entity_num = number;
        }
    }
}

impl RailHost for WeaponHitscanHost {
    fn trace_hit(
        &mut self,
        driver: &mut dyn Q3Driver,
        start: Vec3,
        end: Vec3,
        pass: Option<&ActorId>,
    ) -> Q3TraceResult {
        self.trace(driver, start, end, pass)
    }

    fn hit_target(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<BulletTarget> {
        WeaponHitscanHost::hit_target(self, driver, actor)
    }

    fn is_alive(&mut self, driver: &mut dyn Q3Driver) -> bool {
        driver.native_slot(&self.attacker) == Some(self.entity)
    }

    fn unlink_actor(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) -> Option<ActorId> {
        (self.unlink)(driver, actor)
    }

    fn restore_actor(&mut self, driver: &mut dyn Q3Driver, actor: &ActorId) {
        (self.relink)(driver, actor);
    }

    fn emit_trail(&mut self, driver: &mut dyn Q3Driver, shot: &RailShot) {
        let client_number = driver
            .pool()
            .entity(self.entity)
            .map(|entity| entity.s.client_num)
            .unwrap_or(0);
        let event = driver.pool().temp_entity(shot.end, Q3EntityEvent::Railtrail);
        if let Some(event) = driver.pool().entity_mut(event) {
            event.s.client_num = client_number;
            event.s.origin2 = shot.start;
            event.s.event_parm = shot
                .impact_normal
                .map_or(255, |normal| direction_to_byte(Some(normal)) as i32);
        }
    }

    fn invulnerability_impact(
        &mut self,
        driver: &mut dyn Q3Driver,
        target: &ActorId,
        direction: Vec3,
        point: Vec3,
    ) -> InvulnerabilityImpact {
        WeaponHitscanHost::invulnerability_impact(self, driver, target, direction, point)
            .unwrap_or_else(|error| panic!("{error}"))
    }
}

/// Weapon runtime (`WeaponRuntime`).
#[allow(clippy::type_complexity)]
pub struct WeaponRuntime {
    /// Quad factor.
    pub quad_factor: f32,
    /// Damage factor override.
    pub damage_factor: Option<Rc<dyn Fn(&mut dyn Q3Driver, usize) -> f32>>,
    launcher: Box<dyn MissileLauncher>,
    unlink: WeaponUnlink,
    relink: WeaponRelink,
}

impl WeaponRuntime {
    /// New runtime.
    pub fn new(
        quad_factor: f32,
        launcher: Box<dyn MissileLauncher>,
        unlink: WeaponUnlink,
        relink: WeaponRelink,
    ) -> Self {
        Self {
            quad_factor,
            damage_factor: None,
            launcher,
            unlink,
            relink,
        }
    }

    fn owned(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        if driver.pool().entity(slot).is_none() {
            return Err(failure("Weapon entity does not belong to this pool"));
        }
        Ok(())
    }

    fn quad(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<f32, Q3GameError> {
        if let Some(damage_factor) = &self.damage_factor {
            return Ok(damage_factor(driver, slot));
        }
        let client = client_of(driver.pool(), slot)?;
        let product = driver.combat().product();
        let snapshot = {
            let client_ref = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
            client_ref.clone()
        };
        let item_index = snapshot
            .persistant_powerup
            .and_then(|slot| driver.pool().entity(slot).and_then(|entity| entity.item));
        let tag = item_index.and_then(|index| driver.item_at(index)).map(|item| item.tag);
        Ok(q3_weapon_damage_factor(&snapshot, self.quad_factor, tag, product))
    }

    fn attack(&self, driver: &mut dyn Q3Driver, slot: usize, quad: f32) -> Result<BulletAttack, Q3GameError> {
        let client = client_of(driver.pool(), slot)?;
        let (viewangles, viewheight) = {
            let client_ref = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
            (client_ref.ps.viewangles, client_ref.ps.viewheight)
        };
        let pos_base = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            entity.s.pos.base
        };
        let vectors = angle_vectors(viewangles);
        let eye = vec3(pos_base.x, pos_base.y, pos_base.z + viewheight);
        Ok(BulletAttack {
            forward: vectors.forward,
            right: vectors.right,
            up: vectors.up,
            muzzle: snap_vector(add3(eye, scale3(vectors.forward, 14.0))),
            quad,
        })
    }

    fn scaled(&self, amount: i32, quad: f32) -> i32 {
        qvm_float_to_int(amount as f32 * quad)
    }

    fn contact_host(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        method: i32,
    ) -> Result<WeaponHitscanHost, Q3GameError> {
        let entity = driver
            .pool()
            .entity(slot)
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        Ok(WeaponHitscanHost {
            entity: slot,
            attacker: entity.actor.clone(),
            firing_weapon: entity.s.weapon,
            method,
            unlink: Rc::clone(&self.unlink),
            relink: Rc::clone(&self.relink),
        })
    }

    /// Gauntlet contact check (`checkGauntletAttack`).
    pub fn check_gauntlet_attack(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<bool, Q3GameError> {
        self.owned(driver, slot)?;
        let mut host = self.contact_host(driver, slot, 2)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        let client = client_of(driver.pool(), slot)?;
        let quad = self.quad(driver, slot)?;
        let mut attack = self.attack(driver, slot, quad)?;
        let has_quad = driver
            .pool()
            .client(client)
            .is_some_and(|client| client.ps.powerups.get(Q3Powerup::Quad as usize) != 0);
        Ok(driver.gauntlet_attack(&mut host, &shooter, &mut attack, has_quad))
    }

    fn bullet(
        &self,
        driver: &mut dyn Q3Driver,
        slot: usize,
        attack: &mut BulletAttack,
        spread: i32,
        amount: i32,
    ) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 3)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        driver.bullet_fire(&mut host, &shooter, attack, spread, amount);
        Ok(())
    }

    fn shotgun(&self, driver: &mut dyn Q3Driver, slot: usize, attack: &mut BulletAttack) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 1)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        driver.shotgun_fire(&mut host, &shooter, attack);
        Ok(())
    }

    fn railgun(&self, driver: &mut dyn Q3Driver, slot: usize, attack: &mut BulletAttack) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 10)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        let hits = driver.rail_fire(&mut host, &shooter, attack);
        if driver.native_slot(&shooter) != Some(slot) {
            return Ok(());
        }
        let client = client_of(driver.pool(), slot)?;
        let (accurate_count, accuracy_hits, impressive, reward_until) = {
            let client_ref = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
            (
                client_ref.accurate_count,
                client_ref.accuracy_hits,
                client_ref
                    .ps
                    .persistant
                    .get(Q3PersistentIndex::ImpressiveCount as i32 as usize),
                client_ref.reward_time,
            )
        };
        let time = driver.combat().time();
        let state = driver.rail_statistics(
            &RailStatistics {
                streak: accurate_count,
                hits: accuracy_hits,
                impressive_count: impressive,
                reward_until,
            },
            hits,
            time,
        );
        if let Some(client_ref) = driver.pool().client_mut(client) {
            client_ref.accurate_count = state.streak;
            client_ref.accuracy_hits = state.hits;
            if state.awarded {
                client_ref.ps.persistant.set(
                    Q3PersistentIndex::ImpressiveCount as i32 as usize,
                    state.impressive_count,
                );
                client_ref.ps.e_flags = (client_ref.ps.e_flags & !AWARD_FLAGS) | 0x8000;
                client_ref.reward_time = state.reward_until;
            }
        }
        if state.awarded {
            driver.pool().rankings().reward(slot as i32, 0x8000);
        }
        Ok(())
    }

    fn lightning(&self, driver: &mut dyn Q3Driver, slot: usize, attack: &mut BulletAttack) -> Result<(), Q3GameError> {
        let mut host = self.contact_host(driver, slot, 11)?;
        let shooter = driver
            .pool()
            .entity(slot)
            .map(|entity| entity.actor.clone())
            .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
        driver.lightning_fire(&mut host, &shooter, attack);
        Ok(())
    }

    /// Fire the current weapon (`fire`).
    pub fn fire(&mut self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.owned(driver, slot)?;
        let client = client_of(driver.pool(), slot)?;
        let quad = self.quad(driver, slot)?;
        let weapon = driver.pool().entity(slot).map(|entity| entity.s.weapon).unwrap_or(0);
        driver.pool().rankings().fire_weapon(slot as i32, weapon);
        if weapon != Q3Weapon::GrapplingHook as i32 && weapon != Q3Weapon::Gauntlet as i32 {
            let product = driver.combat().product();
            if let Some(client_ref) = driver.pool().client_mut(client) {
                let shots = if product == Q3Product::Missionpack && weapon == Q3Weapon::Nailgun as i32 {
                    15
                } else {
                    1
                };
                client_ref.accuracy_shots = client_ref.accuracy_shots.wrapping_add(shots);
            }
        }
        let mut attack = self.attack(driver, slot, quad)?;
        if weapon == Q3Weapon::Gauntlet as i32 {
            return Ok(());
        }
        if weapon == Q3Weapon::Lightning as i32 {
            return self.lightning(driver, slot, &mut attack);
        }
        if weapon == Q3Weapon::Shotgun as i32 {
            return self.shotgun(driver, slot, &mut attack);
        }
        if weapon == Q3Weapon::Machinegun as i32 {
            let amount = if driver.combat().game_type() == Q3GameType::Team as i32 {
                5
            } else {
                7
            };
            return self.bullet(driver, slot, &mut attack, 200, amount);
        }
        if weapon == Q3Weapon::GrenadeLauncher as i32 {
            attack.forward = normalize3(vec3(attack.forward.x, attack.forward.y, attack.forward.z + 0.2));
            let projectile = self.launcher.fire_grenade(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::RocketLauncher as i32 {
            let projectile = self.launcher.fire_rocket(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Plasmagun as i32 {
            let projectile = self.launcher.fire_plasma(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Railgun as i32 {
            return self.railgun(driver, slot, &mut attack);
        }
        if weapon == Q3Weapon::Bfg as i32 {
            let projectile = self.launcher.fire_bfg(driver, slot, attack.muzzle, attack.forward);
            let (damage, splash) = driver
                .pool()
                .entity(projectile)
                .map(|entity| (entity.damage, entity.splash_damage))
                .unwrap_or((0, 0));
            if let Some(entity) = driver.pool().entity_mut(projectile) {
                entity.damage = self.scaled(damage, quad);
                entity.splash_damage = self.scaled(splash, quad);
            }
            return Ok(());
        }
        if weapon == Q3Weapon::GrapplingHook as i32 {
            let (fire_held, hook) = {
                let client_ref = driver
                    .pool()
                    .client(client)
                    .ok_or_else(|| failure("Weapon attack requires a client entity"))?;
                (client_ref.fire_held, client_ref.hook)
            };
            if !fire_held && hook.is_none() {
                self.launcher.fire_grapple(driver, slot, attack.muzzle, attack.forward);
            }
            if let Some(client_ref) = driver.pool().client_mut(client) {
                client_ref.fire_held = true;
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Nailgun as i32 {
            if driver.combat().product() == Q3Product::Missionpack {
                for _ in 0..15 {
                    let projectile =
                        self.launcher
                            .fire_nail(driver, slot, attack.muzzle, attack.forward, attack.right, attack.up);
                    let (damage, splash) = driver
                        .pool()
                        .entity(projectile)
                        .map(|entity| (entity.damage, entity.splash_damage))
                        .unwrap_or((0, 0));
                    if let Some(entity) = driver.pool().entity_mut(projectile) {
                        entity.damage = self.scaled(damage, quad);
                        entity.splash_damage = self.scaled(splash, quad);
                    }
                }
            }
            return Ok(());
        }
        if weapon == Q3Weapon::ProxLauncher as i32 {
            if driver.combat().product() == Q3Product::Missionpack {
                attack.forward = normalize3(vec3(attack.forward.x, attack.forward.y, attack.forward.z + 0.2));
                let projectile = self.launcher.fire_prox(driver, slot, attack.muzzle, attack.forward);
                let (damage, splash) = driver
                    .pool()
                    .entity(projectile)
                    .map(|entity| (entity.damage, entity.splash_damage))
                    .unwrap_or((0, 0));
                if let Some(entity) = driver.pool().entity_mut(projectile) {
                    entity.damage = self.scaled(damage, quad);
                    entity.splash_damage = self.scaled(splash, quad);
                }
            }
            return Ok(());
        }
        if weapon == Q3Weapon::Chaingun as i32 {
            if driver.combat().product() == Q3Product::Missionpack {
                return self.bullet(driver, slot, &mut attack, 600, 7);
            }
            return Ok(());
        }
        Ok(())
    }

    /// Start the kamikaze timer (`startKamikaze`), returning the timer slot.
    pub fn start_kamikaze(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<usize, Q3GameError> {
        self.owned(driver, slot)?;
        if driver.combat().product() != Q3Product::Missionpack {
            return Err(failure("Kamikaze requires missionpack"));
        }
        let explosion = driver.pool().spawn_entity()?;
        let time = driver.combat().time();
        if let Some(entity) = driver.pool().entity_mut(explosion) {
            entity.s.e_type = Q3EntityType::Events as i32 + Q3EntityEvent::Kamikaze as i32;
            entity.event_time = time;
        }
        let (client, activator) = {
            let entity = driver
                .pool()
                .entity(slot)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            (entity.client, entity.activator)
        };
        let source_slot = if client.is_some() {
            slot
        } else {
            activator.ok_or_else(|| failure("Kamikaze timer requires its activator"))?
        };
        let source_base = driver
            .pool()
            .entity(source_slot)
            .map(|entity| entity.s.pos.base)
            .unwrap_or_else(|| vec3(0.0, 0.0, 0.0));
        let position = snap_vector(source_base);
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.weapon.kamikazeDamage"))?;
        {
            let entity = driver
                .pool()
                .entity_mut(explosion)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            set_origin(entity, position);
            entity.set_classname(Some("kamikaze".to_string()));
            entity.kamikaze_time = time;
            entity.think = think;
            entity.count = 0;
            entity.movedir = vec3(0.0, 0.0, 0.0);
        }
        driver.pool().set_nextthink(explosion, time.wrapping_add(100));
        driver.world().link(explosion);
        if client.is_some() {
            if let Some(entity) = driver.pool().entity_mut(explosion) {
                entity.activator = Some(slot);
            }
            if let Some(entity) = driver.pool().entity_mut(slot) {
                entity.s.e_flags &= !0x200;
            }
            let target = Participant::Entity(slot);
            driver.combat().damage(
                &target,
                Some(&target.clone()),
                Some(&target),
                None,
                None,
                100_000,
                DamageFlags::NO_PROTECTION,
                MOD_SUICIDE,
            );
        } else {
            let (classname, owner) = {
                let entity = driver
                    .pool()
                    .entity(source_slot)
                    .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
                (entity.classname_value().map(str::to_string), entity.r.owner_num)
            };
            let activator = if classname.as_deref() == Some("bodyque") {
                if owner < 0 || driver.pool().entity(owner as usize).is_none() {
                    return Err(failure("Weapon entity does not belong to this pool"));
                }
                owner as usize
            } else {
                source_slot
            };
            if let Some(entity) = driver.pool().entity_mut(explosion) {
                entity.activator = Some(activator);
            }
        }
        let event = driver.pool().temp_entity(position, Q3EntityEvent::GlobalTeamSound);
        if let Some(event) = driver.pool().entity_mut(event) {
            event.r.sv_flags |= ServerEntityFlags::BROADCAST;
            event.s.event_parm = 13;
        }
        Ok(explosion)
    }

    fn kamikaze_area(
        &self,
        driver: &mut dyn Q3Driver,
        origin: Vec3,
        attacker: Option<&Participant>,
        amount: i32,
        radius: f32,
        shock: bool,
    ) -> Result<(), Q3GameError> {
        let radius = radius.max(1.0);
        let extent = vec3(radius, radius, radius);
        let candidates = driver.world().area_entities(
            &Bounds {
                min: sub3(origin, extent),
                max: add3(origin, extent),
            },
            1024,
        );
        let time = driver.combat().time();
        for number in candidates {
            let (takedamage, kamikaze_time, kamikaze_shock_time, current_origin) = match driver.pool().entity(number) {
                Some(entity) => (
                    entity.takedamage,
                    entity.kamikaze_time,
                    entity.kamikaze_shock_time,
                    entity.r.current_origin,
                ),
                None => return Err(failure(format!("kamikaze area query returned unknown entity {number}"))),
            };
            if shock {
                if kamikaze_shock_time > time {
                    continue;
                }
            } else if !takedamage || kamikaze_time > time {
                continue;
            }
            let Some(link) = driver.world().link_state(number) else {
                return Err(failure("Kamikaze area query returned an unlinked entity"));
            };
            let axis = |value: f32, min: f32, max: f32| -> f32 {
                if value < min {
                    min - value
                } else if value > max {
                    value - max
                } else {
                    0.0
                }
            };
            let dist = length3(vec3(
                axis(origin.x, link.absbounds.min.x, link.absbounds.max.x),
                axis(origin.y, link.absbounds.min.y, link.absbounds.max.y),
                axis(origin.z, link.absbounds.min.z, link.absbounds.max.z),
            ));
            if dist >= radius {
                continue;
            }
            let offset = sub3(current_origin, origin);
            let mut direction = vec3(offset.x, offset.y, offset.z + 24.0);
            let target = Participant::Entity(number);
            driver.combat().damage(
                &target,
                None,
                attacker,
                Some(&mut direction),
                Some(origin),
                amount,
                DamageFlags::RADIUS | DamageFlags::NO_TEAM_PROTECTION,
                MOD_SUICIDE,
            );
            if shock {
                let horizontal = normalize3(vec3(direction.x, direction.y, 0.0));
                let client = driver.pool().entity(number).and_then(|entity| entity.client);
                if let Some(client) = client {
                    if let Some(client) = driver.pool().client_mut(client) {
                        client.ps.velocity = vec3(horizontal.x * 400.0, horizontal.y * 400.0, 100.0);
                    }
                }
                if let Some(entity) = driver.pool().entity_mut(number) {
                    entity.kamikaze_shock_time = time.wrapping_add(3000);
                }
            } else if let Some(entity) = driver.pool().entity_mut(number) {
                entity.kamikaze_time = time.wrapping_add(3000);
            }
        }
        Ok(())
    }

    pub(crate) fn kamikaze_damage(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        let time = driver.combat().time();
        let (count, pos_base, activation, movedir) = {
            let entity = driver
                .pool()
                .entity_mut(slot)
                .ok_or_else(|| failure("Weapon entity does not belong to this pool"))?;
            entity.count = entity.count.wrapping_add(100);
            (
                entity.count,
                entity.s.pos.base,
                entity.activation.clone(),
                entity.movedir,
            )
        };
        if count >= 0 {
            #[allow(clippy::cast_possible_truncation)]
            let radius = count.wrapping_mul(1320) / 2000;
            self.kamikaze_area(driver, pos_base, activation.as_ref(), 25, radius as f32, true)?;
        }
        if count >= 250 {
            #[allow(clippy::cast_possible_truncation)]
            let radius = count.wrapping_sub(250).wrapping_mul(720) / 1750;
            self.kamikaze_area(driver, pos_base, activation.as_ref(), 400, radius as f32, false)?;
        }
        if count >= 2000 {
            driver.pool().free_entity(slot);
            return Ok(());
        }
        driver.pool().set_nextthink(slot, time.wrapping_add(100));
        let angles = vec3(driver.game_crandom() * 2.0, driver.game_crandom() * 2.0, 0.0);
        let short = |angle: f32| -> i32 { qvm_float_to_int(angle * 65536.0 / 360.0) & 65535 };
        for index in 0..MAX_CLIENTS {
            let Some(target) = driver.pool().entity(index) else {
                continue;
            };
            let (inuse, client, ground) = (target.inuse, target.client, target.r.ground.clone());
            let Some(client) = client else { continue };
            if !inuse {
                continue;
            }
            if ground.is_some() {
                let (cx, cy, random) = (driver.game_crandom(), driver.game_crandom(), driver.game_random());
                if let Some(client) = driver.pool().client_mut(client) {
                    client.ps.velocity = vec3(
                        client.ps.velocity.x + cx * 120.0,
                        client.ps.velocity.y + cy * 120.0,
                        30.0 + random * 25.0,
                    );
                }
            }
            let delta = sub3(angles, movedir);
            if let Some(client) = driver.pool().client_mut(client) {
                client.ps.delta_angles[0] = client.ps.delta_angles[0].wrapping_add(short(delta.x));
                client.ps.delta_angles[1] = client.ps.delta_angles[1].wrapping_add(short(delta.y));
                client.ps.delta_angles[2] = client.ps.delta_angles[2].wrapping_add(short(delta.z));
            }
        }
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.movedir = angles;
        }
        Ok(())
    }

    /// Bind weapon save callbacks (`WeaponRuntime` constructor registration).
    pub fn bind_save_callbacks(
        runtime: &Rc<RefCell<WeaponRuntime>>,
        driver: &mut dyn Q3Driver,
    ) -> Result<(), Q3GameError> {
        let kamikaze = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.register(
            "q3.weapon.kamikazeDamage",
            Rc::new(move |driver, slot| {
                let result = kamikaze.borrow().kamikaze_damage(driver, slot);
                or_panic(result);
            }),
        )?;
        Ok(())
    }
}
