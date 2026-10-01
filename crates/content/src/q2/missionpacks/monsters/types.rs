//! Mission-pack monster providers (`src/content/q2/missionpacks/monsters/types.ts`).

use qa_core::identity::ActorId;
use qa_core::math::Vec3;

use crate::q2::foundation::host::{Q2GameServices, Q2Think};

/// Mission-pack selector (`Q2MonsterMissionPack`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Q2MonsterMissionPack {
    /// Xatrix.
    Xatrix,
    /// Rogue.
    Rogue,
}

/// Mission-pack powerup windows (`powerups`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Q2MissionPackPowerups {
    /// Quad damage expiry.
    pub quad_until: f64,
    /// Double damage expiry.
    pub double_until: f64,
    /// Invulnerability expiry.
    pub invulnerability_until: f64,
}

/// Mission-pack monster services (`Q2MissionPackMonsterServices`).
pub trait Q2MissionPackMonsterServices {
    /// Gravity.
    fn gravity(&self) -> f64;
    /// Move an entity linearly toward a destination, then run a think callback.
    fn move_linear(&self, actor: &ActorId, game: &mut Q2GameServices, destination: Vec3, done: Q2Think);
    /// Whether an actor stands in a bad area.
    fn bad_area(&self, actor: &ActorId) -> bool;
    /// Bad-area entity for an actor.
    fn bad_area_entity(&self, actor: &ActorId, origin: Option<Vec3>) -> Option<ActorId>;
    /// Mark a tesla area.
    fn mark_tesla_area(&self, owner: &ActorId, tesla: &ActorId) -> bool;
    /// Powerup windows for an actor.
    fn powerups(&self, actor: &ActorId) -> Q2MissionPackPowerups;
}

/// Mission-pack monster projectile provider (`Q2MissionPackMonsterWeapons`).
pub trait Q2MissionPackMonsterWeapons {
    /// Fire an ion ripper bolt.
    #[allow(clippy::too_many_arguments)]
    fn fire_ion_ripper(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> ActorId;
    /// Fire a blue blaster bolt.
    #[allow(clippy::too_many_arguments)]
    fn fire_blue_blaster(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> ActorId;
    /// Fire a heat-seeking rocket.
    #[allow(clippy::too_many_arguments)]
    fn fire_heat_rocket(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        radius: f64,
        radius_damage: f64,
        turn_fraction: Option<f64>,
    ) -> ActorId;
    /// Fire plasma.
    #[allow(clippy::too_many_arguments)]
    fn fire_plasma(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        radius: f64,
        radius_damage: f64,
    ) -> ActorId;
    /// Fire a blue-blaster variant bolt.
    #[allow(clippy::too_many_arguments)]
    fn fire_blaster2(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        effects: i64,
    ) -> ActorId;
    /// Fire a tracker.
    #[allow(clippy::too_many_arguments)]
    fn fire_tracker(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        enemy: Option<ActorId>,
    ) -> ActorId;
    /// Fire a flechette.
    #[allow(clippy::too_many_arguments)]
    fn fire_flechette(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        damage: f64,
        speed: f64,
        kick: f64,
    ) -> ActorId;
    /// Fire a heat beam.
    #[allow(clippy::too_many_arguments)]
    fn fire_heat_beam(
        &self,
        owner: ActorId,
        game: &mut Q2GameServices,
        start: Vec3,
        direction: Vec3,
        offset: Vec3,
        damage: f64,
        kick: f64,
    );
}

/// Require the registered projectile provider.
pub fn mission_weapons(game: &Q2GameServices) -> std::rc::Rc<dyn Q2MissionPackMonsterWeapons> {
    game.mission_monsters
        .weapons
        .clone()
        .expect("mission-pack monster weapons are not registered")
}

/// Require the registered monster services.
pub fn mission_services(game: &Q2GameServices) -> std::rc::Rc<dyn Q2MissionPackMonsterServices> {
    game.mission_monsters
        .services
        .clone()
        .expect("mission-pack monster services are not registered")
}
