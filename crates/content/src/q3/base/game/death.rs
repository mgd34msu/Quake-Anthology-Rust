//! Quake III base/game: death.
//!
//! Donor provenance: `src/content/q3/base/game/death.ts`.

use qa_core::math::{angle_vectors, length3, scale3, sub3, vec3, vector_to_angles, Vec3};
use qa_core::numeric::qvm_float_to_int;
use std::cell::RefCell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::combat::*;
use crate::q3::base::game::entities::*;
use crate::q3::base::game::format::*;
use crate::q3::base::game::mirrors_game_sim::*;

// ---------------------------------------------------------------------------
// Death and scoring (death.ts).
// ---------------------------------------------------------------------------

/// Corpse contents (`CONTENTS_CORPSE`).
pub(crate) const CONTENTS_CORPSE: i32 = 0x400_0000;

/// Trigger contents (`CONTENTS_TRIGGER`).
pub(crate) const CONTENTS_TRIGGER: i32 = 0x4000_0000;

/// No-drop contents (`CONTENTS_NODROP`).
pub(crate) const CONTENTS_NODROP: i32 = i32::MIN;

/// Kamikaze entity flag (`EF_KAMIKAZE`).
pub(crate) const EF_KAMIKAZE: i32 = 0x200;

/// Ticking entity flag (`EF_TICKING`).
pub(crate) const EF_TICKING: i32 = 0x2;

/// No-draw entity flag (`EF_NODRAW`).
pub(crate) const EF_NODRAW: i32 = 0x80;

/// Award bits mask (`AWARD_MASK`).
pub(crate) const AWARD_MASK: i32 = 0x8 | 0x40 | 0x800 | 0x8000 | 0x10000 | 0x20000;

/// Gauntlet means of death (`MOD_GAUNTLET`).
pub(crate) const MOD_GAUNTLET: i32 = 2;

/// Suicide means of death (`MOD_SUICIDE`).
pub(crate) const MOD_SUICIDE: i32 = 20;

/// Shared means-of-death names (`COMMON_MOD_NAMES`).
pub(crate) const COMMON_MOD_NAMES: [&str; 23] = [
    "MOD_UNKNOWN",
    "MOD_SHOTGUN",
    "MOD_GAUNTLET",
    "MOD_MACHINEGUN",
    "MOD_GRENADE",
    "MOD_GRENADE_SPLASH",
    "MOD_ROCKET",
    "MOD_ROCKET_SPLASH",
    "MOD_PLASMA",
    "MOD_PLASMA_SPLASH",
    "MOD_RAILGUN",
    "MOD_LIGHTNING",
    "MOD_BFG",
    "MOD_BFG_SPLASH",
    "MOD_WATER",
    "MOD_SLIME",
    "MOD_LAVA",
    "MOD_CRUSH",
    "MOD_TELEFRAG",
    "MOD_FALLING",
    "MOD_SUICIDE",
    "MOD_TARGET_LASER",
    "MOD_TRIGGER_HURT",
];

/// Mission-pack means-of-death names.
pub(crate) const MISSIONPACK_MOD_NAMES: [&str; 5] = [
    "MOD_NAIL",
    "MOD_CHAINGUN",
    "MOD_PROXIMITY_MINE",
    "MOD_KAMIKAZE",
    "MOD_JUICED",
];

/// Restore a persistent powerup to the world (`returnQ3PersistentPowerup`).
pub fn return_q3_persistent_powerup(powerup: &EntityRef, link: &dyn Fn(EntityRef)) {
    {
        let mut borrowed = powerup.borrow_mut();
        borrowed.r.sv_flags &= !server_entity_flags::NOCLIENT;
        borrowed.s.e_flags &= !EF_NODRAW;
        borrowed.r.contents = CONTENTS_TRIGGER;
    }
    link(powerup.clone());
}

/// Return a client's carried persistent powerup (`tossQ3ClientPersistentPowerup`).
pub fn toss_q3_client_persistent_powerup(entity: &EntityRef, return_item: &dyn Fn(EntityRef)) {
    let carried = entity
        .borrow_mut()
        .client
        .as_mut()
        .and_then(|client| client.persistant_powerup.take());
    let Some(carried) = carried else {
        return;
    };
    return_item(carried);
    let mut borrowed = entity.borrow_mut();
    if let Some(client) = borrowed.client.as_mut() {
        client.ps.stats.set(MissionpackStatIndex::PersistantPowerup as i32, 0);
    }
}

/// Death frame values (`DeathFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DeathFrame {
    /// Time milliseconds.
    pub time: i32,
    /// Game type.
    pub game_type: i32,
    /// Warmup time.
    pub warmup_time: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Blood enabled.
    pub blood: bool,
}

/// Death item callbacks (`DeathServices.items`).
#[derive(Clone)]
pub struct DeathItemCallbacks {
    /// Item touch callback.
    pub touch_item: TouchCallback,
    /// Dropped flag think callback.
    pub dropped_flag_think: ThinkCallback,
    /// Dropped team item check callback.
    pub check_dropped_team_item: ThinkCallback,
}

/// Mission-pack death services.
#[derive(Clone)]
pub struct MissionpackDeath {
    /// Neutral obelisk lookup.
    pub neutral_obelisk: Rc<dyn Fn() -> Option<EntityRef>>,
    /// Cube timeout seconds.
    pub cube_timeout_seconds: Rc<dyn Fn() -> i32>,
    /// Kamikaze starter.
    pub start_kamikaze: Rc<dyn Fn(EntityRef)>,
}

/// Item drop request context (`DropItemContext` essentials).
#[derive(Clone)]
pub struct SimItemDrop {
    /// Item touch callback.
    pub touch_item: TouchCallback,
    /// Dropped flag think callback.
    pub dropped_flag_think: ThinkCallback,
    /// Dropped team item check callback.
    pub check_dropped_team_item: ThinkCallback,
    /// Time milliseconds.
    pub time: i32,
    /// Unit random draw.
    pub random_unit: Rc<dyn Fn() -> f32>,
}

/// Death host services (`DeathHost`).
#[derive(Clone)]
#[allow(clippy::type_complexity)]
pub struct DeathHost {
    /// Selected character-death path.
    pub character_death_selected: bool,
    /// Death-animation sequence.
    pub death_animations: Rc<RefCell<Q3DeathAnimationSequence>>,
    /// Entity pool.
    pub pool: PoolHandle,
    /// Server world.
    pub world: ServerWorldMirror,
    /// Game random.
    pub random: Rc<RefCell<GameRandomMirror>>,
    /// Shared team scores.
    pub team_scores: Rc<RefCell<[i32; 4]>>,
    /// Hook release.
    pub missiles_hook_free: Rc<dyn Fn(EntityRef)>,
    /// Item callbacks.
    pub items: DeathItemCallbacks,
    /// Frame reader.
    pub frame: Rc<dyn Fn() -> DeathFrame>,
    /// Rank recalculation.
    pub calculate_ranks: Rc<dyn Fn()>,
    /// Scoreboard sender.
    pub send_scoreboard: Rc<dyn Fn(EntityRef)>,
    /// Log sink.
    pub log: Rc<dyn Fn(String)>,
    /// Team frag bonuses.
    pub team_frag_bonuses: Rc<dyn Fn(EntityRef, Option<EntityRef>)>,
    /// Flag returner.
    pub return_flag: Rc<dyn Fn(i32)>,
    /// Product.
    pub product: Q3Product,
    /// Mission-pack services.
    pub missionpack: Option<MissionpackDeath>,
    /// Item drop (`dropItem`, item-motion.ts).
    pub drop_item: Rc<dyn Fn(&SimItemDrop, EntityRef, ItemDefinition, f32) -> EntityRef>,
    /// Item launch (`launchItem`, item-motion.ts).
    pub launch_item: Rc<dyn Fn(&SimItemDrop, ItemDefinition, Vec3, Vec3) -> EntityRef>,
    /// Item table.
    pub item_table: Rc<Q3ItemTable>,
}

/// Panic unless the entity has a client (`clientOf`).
pub(crate) fn require_death_client(entity: &EntityRef) {
    if entity.borrow().client.is_none() {
        panic!("Death operation requires a client entity");
    }
}

/// Death and scoring runtime (`DeathRuntime`).
#[derive(Clone)]
pub struct DeathRuntime {
    /// Host services.
    pub host: DeathHost,
    /// Means-of-death names.
    mod_names: Vec<String>,
}

impl DeathRuntime {
    /// Build a runtime (`new DeathRuntime(host)`).
    #[must_use]
    pub fn new(host: DeathHost) -> Self {
        if host.pool.borrow().options.product != host.product {
            panic!("Death product does not match its entity pool");
        }
        let mut mod_names: Vec<String> = COMMON_MOD_NAMES.iter().map(ToString::to_string).collect();
        if host.product == Q3Product::Missionpack {
            mod_names.extend(MISSIONPACK_MOD_NAMES.iter().map(ToString::to_string));
        }
        mod_names.push("MOD_GRAPPLE".to_string());
        let runtime = Self { host, mod_names };
        runtime.bind_save_callbacks();
        runtime
    }

    /// Item drop context (`dropContext`).
    fn drop_context(&self) -> SimItemDrop {
        let frame = (self.host.frame)();
        let random = self.host.random.clone();
        SimItemDrop {
            touch_item: self.host.items.touch_item.clone(),
            dropped_flag_think: self.host.items.dropped_flag_think.clone(),
            check_dropped_team_item: self.host.items.check_dropped_team_item.clone(),
            time: frame.time,
            random_unit: Rc::new(move || random.borrow_mut().random_value()),
        }
    }

    /// Score-plum event (`scorePlum`).
    pub fn score_plum(&self, entity: &EntityRef, origin: Vec3, score: i32) {
        let number = entity.borrow().s.number;
        let plum = self
            .host
            .pool
            .borrow_mut()
            .temp_entity(origin, EntityEvent::Scoreplum as i32);
        let mut borrowed = plum.borrow_mut();
        borrowed.r.sv_flags |= server_entity_flags::SINGLECLIENT;
        borrowed.r.single_client = number;
        borrowed.s.other_entity_num = number;
        borrowed.s.time = score;
    }

    /// Add score (`addScore`).
    pub fn add_score(&self, entity: &EntityRef, origin: Vec3, score: i32) {
        let frame = (self.host.frame)();
        if entity.borrow().client.is_none() || frame.warmup_time != 0 {
            return;
        }
        self.score_plum(entity, origin, score);
        let team = {
            let mut borrowed = entity.borrow_mut();
            let client = borrowed.client.as_mut().unwrap_or_else(|| {
                panic!("Death operation requires a client entity");
            });
            let score_slot = client.ps.persistant.get(PersistentIndex::Score as i32);
            client
                .ps
                .persistant
                .set(PersistentIndex::Score as i32, score_slot + score);
            client.ps.persistant.get(PersistentIndex::Team as i32)
        };
        if frame.game_type == GameType::Team as i32 {
            let mut scores = self.host.team_scores.borrow_mut();
            scores[team as usize] += score;
        }
        (self.host.calculate_ranks)();
    }

    /// Toss weapon and powerups (`tossClientItems`).
    pub fn toss_client_items(&self, entity: &EntityRef) {
        require_death_client(entity);
        let context = self.drop_context();
        let frame = (self.host.frame)();
        let mut weapon = entity.borrow().s.weapon;
        if weapon == Weapon::Machinegun as i32 || weapon == Weapon::GrapplingHook as i32 {
            let (weapon_state, cmd_weapon, owned) = {
                let borrowed = entity.borrow();
                let client = borrowed.client.as_ref().unwrap_or_else(|| {
                    panic!("Death operation requires a client entity");
                });
                (
                    client.ps.weapon_state,
                    client.pers.cmd.weapon,
                    client.ps.stats.get(stat_schema(self.host.product).weapons),
                )
            };
            if weapon_state == WeaponState::Dropping {
                weapon = cmd_weapon;
            }
            if (owned & 1i32.wrapping_shl(weapon as u32)) == 0 {
                weapon = Weapon::None as i32;
            }
        }
        if weapon > Weapon::Machinegun as i32
            && weapon != Weapon::GrapplingHook as i32
            && entity
                .borrow()
                .client
                .as_ref()
                .map_or(0, |client| client.ps.ammo.get(weapon))
                != 0
        {
            let item = self.host.item_table.find_item_for_weapon(weapon).clone();
            (self.host.drop_item)(&context, entity.clone(), item, 0.0);
        }
        if frame.game_type == GameType::Team as i32 {
            return;
        }
        let mut angle = 45.0f32;
        for powerup in 1..Powerup::NumPowerups as i32 {
            let expires = entity
                .borrow()
                .client
                .as_ref()
                .map_or(0, |client| client.ps.powerups.get(powerup));
            if expires <= context.time {
                continue;
            }
            let Some(item) = self.host.item_table.find_item_for_powerup(powerup).cloned() else {
                continue;
            };
            let dropped = (self.host.drop_item)(&context, entity.clone(), item, angle);
            dropped.borrow_mut().count = 1.max(expires.wrapping_sub(context.time) / 1000);
            angle += 45.0;
        }
    }

    /// Toss harvester cubes (`tossClientCubes`).
    pub fn toss_client_cubes(&self, entity: &EntityRef) {
        let Some(missionpack) = self.host.missionpack.clone() else {
            panic!("TossClientCubes requires missionpack");
        };
        require_death_client(entity);
        let time = (self.host.frame)().time;
        entity
            .borrow_mut()
            .client
            .as_mut()
            .unwrap_or_else(|| panic!("Death operation requires a client entity"))
            .ps
            .generic1 = 0;
        if !self.host.pool.borrow().entities_free() {
            return;
        }
        let team = entity
            .borrow()
            .client
            .as_ref()
            .map_or(Team::Free, |client| client.sess.session_team);
        let item = self
            .host
            .item_table
            .find_item(if team == Team::Red { "Red Cube" } else { "Blue Cube" })
            .unwrap_or_else(|| panic!("Missing source Harvester cube item"))
            .clone();
        let forward = scale3(angle_vectors(vec3(0.0, (time % 360) as f32, 0.0)).forward, 150.0);
        let lift = 200.0 + self.host.random.borrow_mut().crandom_value() * 50.0;
        let velocity = vec3(forward.x, forward.y, forward.z + lift);
        let obelisk = (missionpack.neutral_obelisk)();
        let origin = obelisk.map_or(vec3(0.0, 0.0, 0.0), |obelisk| {
            let base = obelisk.borrow().s.pos.base;
            vec3(base.x, base.y, base.z + 44.0)
        });
        let dropped = (self.host.launch_item)(&self.drop_context(), item, origin, velocity);
        {
            let mut borrowed = dropped.borrow_mut();
            borrowed.nextthink = time.wrapping_add((missionpack.cube_timeout_seconds)().wrapping_mul(1000));
            borrowed.think = self
                .host
                .pool
                .borrow()
                .callbacks
                .borrow()
                .think
                .resolve(Some("q3.base.game.death.tossClientCubes.think"));
            borrowed.spawnflags = team as i32;
        }
    }

    /// Toss persistent powerups (`tossClientPersistantPowerups`).
    pub fn toss_client_persistant_powerups(&self, entity: &EntityRef) {
        if self.host.product != Q3Product::Missionpack {
            panic!("Persistent powerup tossing requires missionpack");
        }
        let world = self.host.world.clone();
        toss_q3_client_persistent_powerup(entity, &|powerup| {
            return_q3_persistent_powerup(&powerup, &|entity| (world.link)(entity));
        });
    }

    /// Face the killer (`lookAtKiller`).
    pub fn look_at_killer(
        &self,
        this: &EntityRef,
        inflictor: Option<&DamageParticipant>,
        attacker: Option<&DamageParticipant>,
    ) {
        let pick = |participant: Option<&DamageParticipant>| -> bool {
            participant.is_some_and(|candidate| !participant_is_entity(candidate, this))
        };
        let target = if pick(attacker) {
            attacker
        } else if pick(inflictor) {
            inflictor
        } else {
            None
        };
        let target_origin = |participant: &DamageParticipant| -> Option<Vec3> {
            match participant {
                DamageParticipant::Shared(shared) => shared.origin,
                DamageParticipant::Native(entity) => Some(entity.borrow().s.pos.base),
            }
        };
        let origin = target.and_then(target_origin);
        let fallback = if origin.is_none() && pick(inflictor) {
            inflictor.and_then(target_origin)
        } else {
            origin
        };
        let (base, angles_y) = {
            let borrowed = this.borrow();
            (borrowed.s.pos.base, borrowed.s.angles.y)
        };
        let yaw = fallback.map_or(angles_y, |point| vector_to_angles(sub3(point, base)).y);
        require_death_client(this);
        this.borrow_mut()
            .client
            .as_mut()
            .unwrap_or_else(|| panic!("Death operation requires a client entity"))
            .ps
            .stats
            .set(stat_schema(self.host.product).dead_yaw, qvm_float_to_int(yaw));
    }

    /// Gib an entity (`gibEntity`).
    pub fn gib_entity(&self, this: &EntityRef, killer: i32) {
        if (this.borrow().s.e_flags & EF_KAMIKAZE) != 0 {
            for index in 0..MAX_GENTITIES {
                let timer = self.host.pool.borrow().at(index as i32);
                let matches = {
                    let borrowed = timer.borrow();
                    borrowed.inuse
                        && borrowed
                            .activator
                            .as_ref()
                            .is_some_and(|activator| Rc::ptr_eq(activator, this))
                        && borrowed.classname.as_deref() == Some("kamikaze timer")
                };
                if matches {
                    self.host.pool.borrow().free(&timer);
                    break;
                }
            }
        }
        self.host
            .pool
            .borrow()
            .add_event(this, EntityEvent::GibPlayer as i32, killer);
        let mut borrowed = this.borrow_mut();
        borrowed.takedamage = false;
        borrowed.s.e_type = EntityType::Invisible as i32;
        borrowed.r.contents = 0;
    }

    /// Corpse die callback (`bodyDie`).
    pub fn body_die(&self, this: &EntityRef) {
        if this.borrow().health > GIB_HEALTH {
            return;
        }
        if !(self.host.frame)().blood {
            this.borrow_mut().health = GIB_HEALTH + 1;
            return;
        }
        self.gib_entity(this, 0);
    }

    /// Kamikaze death timer (`kamikazeDeathTimer`).
    fn kamikaze_death_timer(&self, this: &EntityRef) {
        if self.host.product != Q3Product::Missionpack {
            panic!("Kamikaze death timer requires missionpack");
        }
        let timer = self.host.pool.borrow_mut().spawn();
        {
            let base = this.borrow().s.pos.base;
            let mut borrowed = timer.borrow_mut();
            borrowed.classname = Some("kamikaze timer".to_string());
            borrowed.s.pos.base = base;
            borrowed.r.sv_flags |= server_entity_flags::NOCLIENT;
            borrowed.think = self
                .host
                .pool
                .borrow()
                .callbacks
                .borrow()
                .think
                .resolve(Some("q3.base.game.death.kamikazeDeathTimer.think"));
            borrowed.nextthink = (self.host.frame)().time.wrapping_add(5000);
            borrowed.activator = Some(this.clone());
        }
    }

    /// Almost-score reward (`almostReward`).
    fn almost_reward(&self, this: &EntityRef, attacker: Option<&DamageParticipant>) {
        require_death_client(this);
        this.borrow_mut()
            .client
            .as_mut()
            .unwrap_or_else(|| panic!("Death operation requires a client entity"))
            .ps
            .persistant
            .set(
                PersistentIndex::PlayerEvents as i32,
                this.borrow().client.as_ref().map_or(0, |client| {
                    client.ps.persistant.get(PersistentIndex::PlayerEvents as i32)
                }) ^ 4,
            );
        let Some(attacker) = attacker else {
            panic!("Source almost-score reward dereferences a null attacker");
        };
        if let DamageParticipant::Native(entity) = attacker {
            if entity.borrow().client.is_some() {
                let mut borrowed = entity.borrow_mut();
                if let Some(client) = borrowed.client.as_mut() {
                    let events = client.ps.persistant.get(PersistentIndex::PlayerEvents as i32);
                    client
                        .ps
                        .persistant
                        .set(PersistentIndex::PlayerEvents as i32, events ^ 4);
                }
            }
        }
    }

    /// Almost-capture check (`checkAlmostCapture`).
    fn check_almost_capture(&self, this: &EntityRef, attacker: Option<&DamageParticipant>) {
        require_death_client(this);
        let (has_flag, team, origin) = {
            let borrowed = this.borrow();
            let client = borrowed.client.as_ref().unwrap_or_else(|| {
                panic!("Death operation requires a client entity");
            });
            (
                client.ps.powerups.get(Powerup::Redflag as i32) != 0
                    || client.ps.powerups.get(Powerup::Blueflag as i32) != 0
                    || client.ps.powerups.get(Powerup::Neutralflag as i32) != 0,
                client.sess.session_team,
                client.ps.origin,
            )
        };
        if !has_flag {
            return;
        }
        let blue = team == Team::Blue;
        let ctf = (self.host.frame)().game_type == GameType::Ctf as i32;
        let classname = if ctf == blue {
            "team_CTF_blueflag"
        } else {
            "team_CTF_redflag"
        };
        let mut goal: Option<EntityRef> = None;
        loop {
            goal = find_entity(
                &self.host.pool.borrow(),
                goal.as_ref(),
                EntityStringField::Classname,
                classname,
            );
            match goal.as_ref() {
                None => return,
                Some(found) if (found.borrow().flags & game_flags::DROPPED_ITEM) == 0 => break,
                Some(found) => goal = Some(found.clone()),
            }
        }
        let goal = goal.unwrap_or_else(|| self.host.pool.borrow().at(0));
        let clear = (goal.borrow().r.sv_flags & server_entity_flags::NOCLIENT) == 0
            && length3(sub3(origin, goal.borrow().s.origin)) < 200.0;
        if clear {
            self.almost_reward(this, attacker);
        }
    }

    /// Almost-scored check (`checkAlmostScored`).
    fn check_almost_scored(&self, this: &EntityRef, attacker: Option<&DamageParticipant>) {
        require_death_client(this);
        let (generic1, team, origin) = {
            let borrowed = this.borrow();
            let client = borrowed.client.as_ref().unwrap_or_else(|| {
                panic!("Death operation requires a client entity");
            });
            (client.ps.generic1, client.sess.session_team, client.ps.origin)
        };
        if generic1 == 0 {
            return;
        }
        let classname = if team == Team::Blue {
            "team_redobelisk"
        } else {
            "team_blueobelisk"
        };
        let goal = find_entity(&self.host.pool.borrow(), None, EntityStringField::Classname, classname);
        if goal
            .as_ref()
            .is_some_and(|goal| length3(sub3(origin, goal.borrow().s.origin)) < 200.0)
        {
            self.almost_reward(this, attacker);
        }
    }

    /// Carried flag (`carriedFlag`).
    fn carried_flag(&self, this: &EntityRef) -> Option<(i32, i32)> {
        require_death_client(this);
        let borrowed = this.borrow();
        let client = borrowed.client.as_ref().unwrap_or_else(|| {
            panic!("Death operation requires a client entity");
        });
        if client.ps.powerups.get(Powerup::Neutralflag as i32) != 0 {
            Some((Team::Free as i32, Powerup::Neutralflag as i32))
        } else if client.ps.powerups.get(Powerup::Redflag as i32) != 0 {
            Some((Team::Red as i32, Powerup::Redflag as i32))
        } else if client.ps.powerups.get(Powerup::Blueflag as i32) != 0 {
            Some((Team::Blue as i32, Powerup::Blueflag as i32))
        } else {
            None
        }
    }

    /// Player death (`playerDie`).
    pub fn player_die(
        &self,
        this: &EntityRef,
        inflictor: Option<&DamageParticipant>,
        attacker: Option<&DamageParticipant>,
        _damage: i32,
        means_of_death: i32,
    ) {
        require_death_client(this);
        let frame = (self.host.frame)();
        {
            let borrowed = this.borrow();
            let client = borrowed.client.as_ref().unwrap_or_else(|| {
                panic!("Death operation requires a client entity");
            });
            if client.ps.pm_type == MoveType::Dead || frame.intermission_time != 0 {
                return;
            }
        }
        self.check_almost_capture(this, attacker);
        self.check_almost_scored(this, attacker);
        let hook = this.borrow().client.as_ref().and_then(|client| client.hook.clone());
        if let Some(hook) = hook {
            (self.host.missiles_hook_free)(hook);
        }
        if self.host.product == Q3Product::Missionpack {
            let (ticking, activator) = {
                let borrowed = this.borrow();
                (
                    borrowed.client.as_ref().map_or(0, |client| client.ps.e_flags) & EF_TICKING != 0,
                    borrowed.activator.clone(),
                )
            };
            if ticking && activator.is_some() {
                let activator = activator.unwrap_or_else(|| this.clone());
                this.borrow_mut()
                    .client
                    .as_mut()
                    .unwrap_or_else(|| panic!("Death operation requires a client entity"))
                    .ps
                    .e_flags &= !EF_TICKING;
                activator.borrow_mut().think = self
                    .host
                    .pool
                    .borrow()
                    .callbacks
                    .borrow()
                    .think
                    .resolve(Some("q3.base.game.death.playerDie.think"));
                activator.borrow_mut().nextthink = frame.time;
            }
        }
        this.borrow_mut()
            .client
            .as_mut()
            .unwrap_or_else(|| panic!("Death operation requires a client entity"))
            .ps
            .pm_type = MoveType::Dead;
        let victim_number = this.borrow().s.number;
        let victim_origin = this.borrow().r.current_origin;
        let victim_name = this
            .borrow()
            .client
            .as_ref()
            .map_or(String::new(), |client| client.pers.netname.clone());
        let mut killer = attacker.map_or(ENTITYNUM_WORLD, |attacker| match attacker {
            DamageParticipant::Native(entity) => entity.borrow().s.number,
            DamageParticipant::Shared(_) => ENTITYNUM_WORLD,
        });
        let mut killer_name = match attacker {
            None => "<world>".to_string(),
            Some(DamageParticipant::Native(entity)) => entity
                .borrow()
                .client
                .as_ref()
                .map_or("<non-client>".to_string(), |client| client.pers.netname.clone()),
            Some(DamageParticipant::Shared(_)) => "<non-client>".to_string(),
        };
        if killer < 0 || killer >= MAX_CLIENTS as i32 {
            killer = ENTITYNUM_WORLD;
            killer_name = "<world>".to_string();
        }
        self.host.pool.borrow().rankings.borrow_mut().player_die(
            this.borrow().slot as i32,
            killer,
            ranked_means_of_death(self.host.product, means_of_death),
        );
        let obituary = self
            .mod_names
            .get(means_of_death as usize)
            .cloned()
            .unwrap_or_else(|| "<bad obituary>".to_string());
        (self.host.log)(game_format(
            "Kill: %i %i %i: %s killed %s by %s\n",
            &[
                GameFormatArgument::Int(killer),
                GameFormatArgument::Int(victim_number),
                GameFormatArgument::Int(means_of_death),
                GameFormatArgument::Text(killer_name),
                GameFormatArgument::Text(victim_name),
                GameFormatArgument::Text(obituary),
            ],
        ));
        let obituary_event = self
            .host
            .pool
            .borrow_mut()
            .temp_entity(victim_origin, EntityEvent::Obituary as i32);
        {
            let mut borrowed = obituary_event.borrow_mut();
            borrowed.s.event_parm = means_of_death;
            borrowed.s.other_entity_num = victim_number;
            borrowed.s.other_entity_num2 = killer;
            borrowed.r.sv_flags = server_entity_flags::BROADCAST;
        }
        this.borrow_mut()
            .client
            .as_mut()
            .unwrap_or_else(|| panic!("Death operation requires a client entity"))
            .ps
            .persistant
            .set(
                PersistentIndex::Killed as i32,
                this.borrow()
                    .client
                    .as_ref()
                    .map_or(0, |client| client.ps.persistant.get(PersistentIndex::Killed as i32))
                    + 1,
            );
        let attacker_native = match attacker {
            Some(DamageParticipant::Native(entity)) if entity.borrow().client.is_some() => Some(entity.clone()),
            _ => None,
        };
        if let Some(killer_entity) = attacker_native.clone() {
            killer_entity
                .borrow_mut()
                .client
                .as_mut()
                .unwrap_or_else(|| panic!("Death operation requires a client entity"))
                .last_killed_client = victim_number;
            let same_team = {
                let victim_team = this
                    .borrow()
                    .client
                    .as_ref()
                    .map_or(Team::Free, |client| client.sess.session_team);
                let killer_team = killer_entity
                    .borrow()
                    .client
                    .as_ref()
                    .map_or(Team::Free, |client| client.sess.session_team);
                frame.game_type >= GameType::Team as i32 && victim_team == killer_team
            };
            if Rc::ptr_eq(&killer_entity, this) || same_team {
                self.add_score(&killer_entity, victim_origin, -1);
            } else {
                self.add_score(&killer_entity, victim_origin, 1);
                if means_of_death == MOD_GAUNTLET {
                    {
                        let mut borrowed = killer_entity.borrow_mut();
                        let client = borrowed.client.as_mut().unwrap_or_else(|| {
                            panic!("Death operation requires a client entity");
                        });
                        let count = client.ps.persistant.get(PersistentIndex::GauntletFragCount as i32);
                        client
                            .ps
                            .persistant
                            .set(PersistentIndex::GauntletFragCount as i32, count + 1);
                        client.ps.e_flags = (client.ps.e_flags & !AWARD_MASK) | 0x40;
                        client.reward_time = frame.time.wrapping_add(2000);
                    }
                    {
                        let mut borrowed = this.borrow_mut();
                        let client = borrowed.client.as_mut().unwrap_or_else(|| {
                            panic!("Death operation requires a client entity");
                        });
                        let events = client.ps.persistant.get(PersistentIndex::PlayerEvents as i32);
                        client
                            .ps
                            .persistant
                            .set(PersistentIndex::PlayerEvents as i32, events ^ 2);
                    }
                }
                let recent = frame.time.wrapping_sub(
                    killer_entity
                        .borrow()
                        .client
                        .as_ref()
                        .map_or(0, |client| client.last_kill_time),
                ) < 3000;
                if recent {
                    self.host
                        .pool
                        .borrow()
                        .rankings
                        .borrow_mut()
                        .reward(killer_entity.borrow().slot as i32, 0x8);
                    let mut borrowed = killer_entity.borrow_mut();
                    let client = borrowed.client.as_mut().unwrap_or_else(|| {
                        panic!("Death operation requires a client entity");
                    });
                    let count = client.ps.persistant.get(PersistentIndex::ExcellentCount as i32);
                    client
                        .ps
                        .persistant
                        .set(PersistentIndex::ExcellentCount as i32, count + 1);
                    client.ps.e_flags = (client.ps.e_flags & !AWARD_MASK) | 0x8;
                    client.reward_time = frame.time.wrapping_add(2000);
                }
                killer_entity
                    .borrow_mut()
                    .client
                    .as_mut()
                    .unwrap_or_else(|| panic!("Death operation requires a client entity"))
                    .last_kill_time = frame.time;
            }
        } else {
            self.add_score(this, victim_origin, -1);
        }
        if attacker.is_none() || matches!(attacker, Some(DamageParticipant::Native(_))) {
            let native = match attacker {
                Some(DamageParticipant::Native(entity)) => Some(entity.clone()),
                _ => None,
            };
            (self.host.team_frag_bonuses)(this.clone(), native);
        }
        if means_of_death == MOD_SUICIDE {
            if let Some((team, powerup)) = self.carried_flag(this) {
                (self.host.return_flag)(team);
                this.borrow_mut()
                    .client
                    .as_mut()
                    .unwrap_or_else(|| panic!("Death operation requires a client entity"))
                    .ps
                    .powerups
                    .set(powerup, 0);
            }
        }
        let contents = (self.host.world.point_contents)(victim_origin, -1);
        if (contents & CONTENTS_NODROP) == 0 {
            self.toss_client_items(this);
        } else if let Some((team, _)) = self.carried_flag(this) {
            (self.host.return_flag)(team);
        }
        if self.host.product == Q3Product::Missionpack {
            self.toss_client_persistant_powerups(this);
            if frame.game_type == GameType::Harvester as i32 {
                self.toss_client_cubes(this);
            }
        }
        (self.host.send_scoreboard)(this.clone());
        let max_clients = self.host.pool.borrow().max_clients();
        for index in 0..max_clients {
            let follower = self.host.pool.borrow().client_at(index as i32);
            if follower.pers.connected == ConnectionState::Connected
                && follower.sess.session_team == Team::Spectator
                && follower.sess.spectator_client == victim_number
            {
                (self.host.send_scoreboard)(self.host.pool.borrow().at(index as i32));
            }
        }
        if self.host.character_death_selected {
            {
                let mut borrowed = this.borrow_mut();
                borrowed.s.weapon = Weapon::None as i32;
                borrowed.s.powerups = 0;
                borrowed.s.loop_sound = 0;
                let client = borrowed.client.as_mut().unwrap_or_else(|| {
                    panic!("Death operation requires a client entity");
                });
                client.respawn_time = frame.time.wrapping_add(1700);
                for index in 0..client.ps.powerups.len() {
                    client.ps.powerups.set(index as i32, 0);
                }
            }
            return;
        }
        {
            let mut borrowed = this.borrow_mut();
            borrowed.takedamage = true;
            borrowed.s.weapon = Weapon::None as i32;
            borrowed.s.powerups = 0;
            borrowed.r.contents = CONTENTS_CORPSE;
            let yaw = borrowed.s.angles.y;
            borrowed.s.angles = vec3(0.0, yaw, 0.0);
        }
        self.look_at_killer(this, inflictor, attacker);
        {
            let mut borrowed = this.borrow_mut();
            let angles = borrowed.s.angles;
            let client = borrowed.client.as_mut().unwrap_or_else(|| {
                panic!("Death operation requires a client entity");
            });
            client.ps.viewangles = angles;
            borrowed.s.loop_sound = 0;
            let maxs = borrowed.r.maxs;
            borrowed.r.maxs = vec3(maxs.x, maxs.y, -8.0);
            let client = borrowed.client.as_mut().unwrap_or_else(|| {
                panic!("Death operation requires a client entity");
            });
            client.respawn_time = frame.time.wrapping_add(1700);
            for index in 0..client.ps.powerups.len() {
                client.ps.powerups.set(index as i32, 0);
            }
        }
        let health = this.borrow().health;
        if (health <= GIB_HEALTH && (contents & CONTENTS_NODROP) == 0 && frame.blood) || means_of_death == MOD_SUICIDE {
            self.gib_entity(this, killer);
        } else {
            let (animation, event) = self.host.death_animations.borrow_mut().next();
            {
                let mut borrowed = this.borrow_mut();
                if borrowed.health <= GIB_HEALTH {
                    borrowed.health = GIB_HEALTH + 1;
                }
                let client = borrowed.client.as_mut().unwrap_or_else(|| {
                    panic!("Death operation requires a client entity");
                });
                client.ps.legs_anim = ((client.ps.legs_anim & 128) ^ 128) | animation as i32;
                client.ps.torso_anim = ((client.ps.torso_anim & 128) ^ 128) | animation as i32;
            }
            self.host.pool.borrow().add_event(this, event as i32, killer);
            this.borrow_mut().die = self
                .host
                .pool
                .borrow()
                .callbacks
                .borrow()
                .die
                .resolve(Some("q3.death.body"));
            if self.host.product == Q3Product::Missionpack && (this.borrow().s.e_flags & EF_KAMIKAZE) != 0 {
                self.kamikaze_death_timer(this);
            }
        }
        (self.host.world.link)(this.clone());
    }

    /// Register saved callbacks (`bindSaveCallbacks`).
    pub fn bind_save_callbacks(&self) {
        let pool = self.host.pool.clone();
        let captured = pool.clone();
        pool.borrow().callbacks.borrow_mut().think.intern(
            "q3.base.game.death.tossClientCubes.think",
            Rc::new(move |entity: EntityRef| {
                captured.borrow().free(&entity);
            }),
        );
        let host = self.host.clone();
        let captured = host.clone();
        host.pool.borrow().callbacks.borrow_mut().think.intern(
            "q3.base.game.death.kamikazeDeathTimer.think",
            Rc::new(move |entity: EntityRef| {
                if captured.product != Q3Product::Missionpack {
                    panic!("Kamikaze callback requires missionpack");
                }
                let Some(missionpack) = captured.missionpack.clone() else {
                    panic!("Kamikaze callback requires missionpack");
                };
                (missionpack.start_kamikaze)(entity.clone());
                captured.pool.borrow().free(&entity);
            }),
        );
        let pool = self.host.pool.clone();
        let captured = pool.clone();
        pool.borrow().callbacks.borrow_mut().think.intern(
            "q3.base.game.death.playerDie.think",
            Rc::new(move |entity: EntityRef| {
                captured.borrow().free(&entity);
            }),
        );
        let runtime = self.clone();
        self.host.pool.borrow().callbacks.borrow_mut().die.register(
            "q3.death.body",
            Rc::new(move |entity, _, _, _, _| runtime.body_die(&entity)),
        );
        let runtime = self.clone();
        self.host.pool.borrow().callbacks.borrow_mut().die.register(
            "q3.death.player",
            Rc::new(move |entity, inflictor, attacker, damage, method| {
                runtime.player_die(&entity, inflictor.as_ref(), attacker.as_ref(), damage, method);
            }),
        );
    }
}
