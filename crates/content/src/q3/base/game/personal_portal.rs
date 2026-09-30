//! Quake III base/game: personal portal.
//!
//! Donor provenance: `src/content/q3/base/game/personal-portal.ts`.

use crate::value::obj;
use crate::value::SaveJson;
use crate::value::SaveReader;
use qa_core::math::Vec3;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::mirrors_game_state::*;
use crate::q3::base::game::mover::*;
use crate::q3::base::game::save_values::*;
use crate::q3::base::game::state::*;
use crate::q3::base::game::utilities::*;

// ---------------------------------------------------------------------------
// personal-portal.ts: missionpack personal portals (g_misc.c)
// ---------------------------------------------------------------------------

/// Corpse contents (`CONTENTS_CORPSE`).
pub(crate) const CONTENTS_CORPSE: i32 = 0x4000000;

/// Trigger contents (`CONTENTS_TRIGGER`).
pub(crate) const CONTENTS_TRIGGER: i32 = 0x40000000;

/// Telefrag means of death (`MOD_TELEFRAG`).
pub(crate) const MOD_TELEFRAG: i32 = 18;

/// Portal health (`PORTAL_HEALTH`).
pub(crate) const PORTAL_HEALTH: i32 = 200;

/// Portal enable delay (`PORTAL_ENABLE_DELAY`).
pub(crate) const PORTAL_ENABLE_DELAY: i32 = 1_000;

/// Portal lifetime (`PORTAL_LIFETIME`).
pub(crate) const PORTAL_LIFETIME: i32 = 2 * 60 * 1_000;

/// Portal destination classname.
pub(crate) const PORTAL_DESTINATION: &str = "hi_portal destination";

/// Portal source classname.
pub(crate) const PORTAL_SOURCE: &str = "hi_portal source";

/// Personal portal runtime (`PersonalPortalRuntime`).
#[derive(Debug, Clone)]
pub struct PersonalPortalRuntime {
    portal_sequence: i32,
}

impl PersonalPortalRuntime {
    /// New runtime.
    #[must_use]
    pub fn new() -> Self {
        Self { portal_sequence: 0 }
    }

    /// Portal sequence.
    #[must_use]
    pub fn portal_sequence(&self) -> i32 {
        self.portal_sequence
    }

    /// Capture save state.
    #[must_use]
    pub fn capture_save_state(&self) -> SaveJson {
        obj(vec![("portalSequence", num_i32(self.portal_sequence))])
    }

    /// Restore save state.
    pub fn restore_save_state(&mut self, value: &SaveJson) -> Result<(), Q3GameError> {
        let reader = SaveReader::at(value, "q3.portals");
        let sequence = reader.field("portalSequence").integer(-2_147_483_648)?;
        if sequence > 2_147_483_647 {
            return Err(reader.fail("portal sequence exceeds source integer range").into());
        }
        #[allow(clippy::cast_possible_truncation)]
        {
            self.portal_sequence = sequence as i32;
        }
        Ok(())
    }

    fn check_host(&self, driver: &mut dyn Q3Driver) -> Result<(), Q3GameError> {
        if driver.combat().product() != Q3Product::Missionpack {
            return Err(failure("Personal portals require a missionpack entity pool"));
        }
        Ok(())
    }

    fn owned(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        if driver.pool().entity(slot).is_none() {
            return Err(failure(
                "Personal portal entity does not belong to its entity pool or was replaced",
            ));
        }
        Ok(())
    }

    fn player(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<usize, Q3GameError> {
        self.owned(driver, slot)?;
        driver
            .pool()
            .entity(slot)
            .and_then(|entity| entity.client)
            .ok_or_else(|| failure("Personal portal use requires a client entity"))
    }

    fn free_portal(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.owned(driver, slot)?;
        driver.pool().free_entity(slot);
        Ok(())
    }

    fn destination(&self, driver: &mut dyn Q3Driver, sequence: i32) -> Option<usize> {
        let mut current = None;
        loop {
            current = find_entity(
                driver.pool(),
                current,
                EntityStringField::Classname,
                Some(PORTAL_DESTINATION),
            );
            let slot = current?;
            if driver
                .pool()
                .entity(slot)
                .is_some_and(|entity| entity.count == sequence)
            {
                return Some(slot);
            }
        }
    }

    fn portal_die(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.free_portal(driver, slot)
    }

    fn drop_carried_flag(&self, driver: &mut dyn Q3Driver, player: usize) -> Result<(), Q3GameError> {
        if driver.map_travel_mode() {
            driver.map_travel_drop_flag(player);
            return Ok(());
        }
        let client = driver
            .pool()
            .entity(player)
            .and_then(|entity| entity.client)
            .ok_or_else(|| failure("Portal touch requires a client entity"))?;
        let powerup = {
            let client = driver
                .pool()
                .client(client)
                .ok_or_else(|| failure("Portal touch requires a client entity"))?;
            if client.ps.powerups.get(Q3Powerup::Neutralflag as usize) != 0 {
                Q3Powerup::Neutralflag as i32
            } else if client.ps.powerups.get(Q3Powerup::Redflag as usize) != 0 {
                Q3Powerup::Redflag as i32
            } else if client.ps.powerups.get(Q3Powerup::Blueflag as usize) != 0 {
                Q3Powerup::Blueflag as i32
            } else {
                Q3Powerup::None as i32
            }
        };
        if powerup == Q3Powerup::None as i32 {
            return Ok(());
        }
        let Some(item) = driver.find_item_for_powerup(powerup) else {
            return Err(failure(format!(
                "Portal carried flag {powerup} is absent from the missionpack item table"
            )));
        };
        driver.drop_item(player, item, 0);
        if let Some(client) = driver.pool().client_mut(client) {
            client.ps.powerups.set(powerup as usize, 0);
        }
        Ok(())
    }

    fn portal_touch(&self, driver: &mut dyn Q3Driver, source: usize, other: usize) -> Result<(), Q3GameError> {
        self.owned(driver, source)?;
        let (health, client) = {
            let entity = driver
                .pool()
                .entity(other)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.health, entity.client)
        };
        if health <= 0 || client.is_none() {
            return Ok(());
        }
        self.owned(driver, other)?;
        self.drop_carried_flag(driver, other)?;
        let count = driver.pool().entity(source).map(|entity| entity.count).unwrap_or(0);
        let destination = self.destination(driver, count);
        let Some(destination) = destination else {
            let (pos1, angles) = {
                let entity = driver.pool().entity(source).ok_or_else(|| {
                    failure("Personal portal entity does not belong to its entity pool or was replaced")
                })?;
                (entity.pos1, entity.s.angles)
            };
            if pos1.x != 0.0 || pos1.y != 0.0 || pos1.z != 0.0 {
                self.teleport(driver, other, pos1, angles);
            }
            let target = Participant::Entity(other);
            driver.combat().damage(
                &target,
                Some(&target.clone()),
                Some(&target),
                None,
                None,
                100_000,
                DamageFlags::NO_PROTECTION,
                MOD_TELEFRAG,
            );
            return Ok(());
        };
        let (origin, angles) = {
            let entity = driver
                .pool()
                .entity(destination)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.s.pos.base, entity.s.angles)
        };
        self.teleport(driver, other, origin, angles);
        Ok(())
    }

    fn teleport(&self, driver: &mut dyn Q3Driver, player: usize, origin: Vec3, angles: Vec3) {
        if driver.map_travel_mode() {
            driver.map_travel_teleport(player, origin, angles);
        } else {
            driver.teleport_player(player, origin, angles);
        }
    }

    fn portal_enable(&self, driver: &mut dyn Q3Driver, slot: usize) -> Result<(), Q3GameError> {
        self.owned(driver, slot)?;
        let touch = driver
            .pool()
            .callbacks()
            .touch
            .resolve(Some("q3.base.game.personal-portal.portalEnable.touch"))?;
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.personal-portal.portalEnable.think"))?;
        if let Some(entity) = driver.pool().entity_mut(slot) {
            entity.touch = touch;
            entity.think = think;
        }
        let time = driver.combat().time();
        driver.pool().set_nextthink(slot, time.wrapping_add(PORTAL_LIFETIME));
        Ok(())
    }

    /// Drop a portal destination (`dropPortalDestination`).
    pub fn drop_portal_destination(&mut self, driver: &mut dyn Q3Driver, player: usize) -> Result<(), Q3GameError> {
        self.check_host(driver)?;
        let client = self.player(driver, player)?;
        let (pos_base, mins, maxs, apos_base) = {
            let entity = driver
                .pool()
                .entity(player)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.s.pos.base, entity.r.mins, entity.r.maxs, entity.s.apos.base)
        };
        let portal = driver.pool().spawn_entity()?;
        let model = driver.model_index(Some("models/powerups/teleporter/tele_exit.md3"));
        let die = driver
            .pool()
            .callbacks()
            .die
            .resolve(Some("q3.base.game.personal-portal.dropPortalDestination.die"))?;
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.personal-portal.portalEnable.think"))?;
        let time = driver.combat().time();
        {
            let entity = driver
                .pool()
                .entity_mut(portal)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            entity.s.modelindex = model;
            set_origin(entity, snap_vector(pos_base));
            entity.r.mins = mins;
            entity.r.maxs = maxs;
            entity.set_classname(Some(PORTAL_DESTINATION.to_string()));
            entity.r.contents = CONTENTS_CORPSE;
            entity.takedamage = true;
            entity.health = PORTAL_HEALTH;
            entity.die = die;
            entity.s.angles = apos_base;
            entity.think = think;
        }
        driver.pool().set_nextthink(portal, time.wrapping_add(PORTAL_LIFETIME));
        driver.world().link(portal);
        self.portal_sequence = self.portal_sequence.wrapping_add(1);
        let sequence = self.portal_sequence;
        if let Some(client) = driver.pool().client_mut(client) {
            client.portal_id = sequence;
        }
        if let Some(entity) = driver.pool().entity_mut(portal) {
            entity.count = sequence;
        }
        let Some(item) = driver.find_item("Portal") else {
            return Err(failure("Portal holdable is absent from the missionpack item table"));
        };
        if item < 1 {
            return Err(failure("Portal holdable has no missionpack item index"));
        }
        if let Some(client) = driver.pool().client_mut(client) {
            let slot = stat_schema(Q3Product::Missionpack).holdable_item;
            client.ps.stats.set(slot, item as i32);
        }
        Ok(())
    }

    /// Drop a portal source (`dropPortalSource`).
    pub fn drop_portal_source(&mut self, driver: &mut dyn Q3Driver, player: usize) -> Result<(), Q3GameError> {
        self.check_host(driver)?;
        let client = self.player(driver, player)?;
        let (pos_base, mins, maxs) = {
            let entity = driver
                .pool()
                .entity(player)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            (entity.s.pos.base, entity.r.mins, entity.r.maxs)
        };
        let portal = driver.pool().spawn_entity()?;
        let model = driver.model_index(Some("models/powerups/teleporter/tele_enter.md3"));
        let die = driver
            .pool()
            .callbacks()
            .die
            .resolve(Some("q3.base.game.personal-portal.dropPortalDestination.die"))?;
        let time = driver.combat().time();
        {
            let entity = driver
                .pool()
                .entity_mut(portal)
                .ok_or_else(|| failure("Personal portal entity does not belong to its entity pool or was replaced"))?;
            entity.s.modelindex = model;
            set_origin(entity, snap_vector(pos_base));
            entity.r.mins = mins;
            entity.r.maxs = maxs;
            entity.set_classname(Some(PORTAL_SOURCE.to_string()));
            entity.r.contents = CONTENTS_CORPSE | CONTENTS_TRIGGER;
            entity.takedamage = true;
            entity.health = PORTAL_HEALTH;
            entity.die = die;
        }
        driver.world().link(portal);
        let portal_id = driver.pool().client(client).map(|client| client.portal_id).unwrap_or(0);
        if let Some(entity) = driver.pool().entity_mut(portal) {
            entity.count = portal_id;
        }
        if let Some(client) = driver.pool().client_mut(client) {
            client.portal_id = 0;
        }
        let think = driver
            .pool()
            .callbacks()
            .think
            .resolve(Some("q3.base.game.personal-portal.dropPortalSource.think"))?;
        if let Some(entity) = driver.pool().entity_mut(portal) {
            entity.think = think;
        }
        driver
            .pool()
            .set_nextthink(portal, time.wrapping_add(PORTAL_ENABLE_DELAY));
        if let Some(destination) = self.destination(driver, portal_id) {
            let origin = driver.pool().entity(destination).map(|entity| entity.s.pos.base);
            if let (Some(origin), Some(entity)) = (origin, driver.pool().entity_mut(portal)) {
                entity.pos1 = origin;
            }
        }
        Ok(())
    }

    /// Bind save callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(
        runtime: &Rc<RefCell<PersonalPortalRuntime>>,
        driver: &mut dyn Q3Driver,
    ) -> Result<(), Q3GameError> {
        let touch_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().touch.intern(
            "q3.base.game.personal-portal.portalEnable.touch",
            Rc::new(move |driver, slot, other, _contact| {
                if let Participant::Entity(other) = other {
                    let result = touch_runtime.borrow().portal_touch(driver, slot, *other);
                    or_panic(result);
                }
            }),
        )?;
        let free_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.intern(
            "q3.base.game.personal-portal.portalEnable.think",
            Rc::new(move |driver, slot| {
                let result = free_runtime.borrow().free_portal(driver, slot);
                or_panic(result);
            }),
        )?;
        let die_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().die.intern(
            "q3.base.game.personal-portal.dropPortalDestination.die",
            Rc::new(move |driver, slot, _inflictor, _attacker, _damage, _method| {
                let result = die_runtime.borrow().portal_die(driver, slot);
                or_panic(result);
            }),
        )?;
        let enable_runtime = Rc::clone(runtime);
        driver.pool().callbacks_mut().think.intern(
            "q3.base.game.personal-portal.dropPortalSource.think",
            Rc::new(move |driver, slot| {
                let result = enable_runtime.borrow().portal_enable(driver, slot);
                or_panic(result);
            }),
        )?;
        Ok(())
    }
}

impl Default for PersonalPortalRuntime {
    fn default() -> Self {
        Self::new()
    }
}
