//! Quake III team-arena: client think.
//!
//! Donor provenance: `src/content/q3/team-arena/client-think.ts`.

use qa_core::identity::ActorId;
use qa_core::math::{add3, sub3, vec3, Bounds, Vec3};
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::state::{ConnectionState, GameFlags, SpectatorState, MAX_CLIENTS, MAX_GENTITIES};
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::ServerEntityFlags;
use crate::q3::base::shared::items::{player_touches_item, Trajectory as ItemsTrajectory};
use crate::q3::base::shared::player_state::{CommandButtons, MoveFlags, UserCommand};
use crate::q3::team_arena::client_effects::*;
use crate::q3::team_arena::movement_host::*;
use crate::q3::team_arena::support::*;

// ---------------------------------------------------------------------------
// client-think.ts
// ---------------------------------------------------------------------------

/// Think frame times (`ClientThinkFrame`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientThinkFrame {
    /// Current time.
    pub time: i32,
    /// Intermission time.
    pub intermission_time: i32,
    /// Queued intermission time.
    pub intermission_queued: i32,
}

/// Think settings (`ClientThinkSettings`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClientThinkSettings {
    /// Move debugging level.
    pub debug_move: i32,
    /// Synchronous clients.
    pub synchronous_clients: bool,
    /// Fixed pmove.
    pub pmove_fixed: bool,
    /// Pmove millisecond step.
    pub pmove_msec: i32,
    /// Gravity.
    pub gravity: f32,
    /// Speed.
    pub speed: f32,
    /// `dmflags` bits.
    pub dmflags: i32,
    /// Smooth clients.
    pub smooth_clients: bool,
    /// Forced respawn delay.
    pub force_respawn_seconds: i32,
    /// Single-player rules.
    pub single_player: bool,
}

/// Touch dispatch (`ClientTouchAccess`).
pub trait TouchAccess {
    /// Native entity for an actor.
    fn native(&self, actor: &ActorId) -> Option<EntityRef>;
    /// Whether an actor is a trigger.
    fn is_trigger(&self, actor: &ActorId) -> bool;
    /// Dispatch a touch.
    fn touch(&self, this: &ActorId, other: &ActorId);
}

/// Client-think services (`ClientThinkHost`).
pub trait ClientThinkHost: MovementHost {
    /// Entity pool.
    fn pool(&self) -> PoolRef;
    /// Server world.
    fn world(&self) -> WorldRef;
    /// Touch dispatch.
    fn touches(&self) -> Rc<dyn TouchAccess>;
    /// Core effect services.
    fn effects(&self) -> EffectsCoreRef;
    /// Item services.
    fn items(&self) -> Rc<dyn ItemHost>;
    /// Optional per-actor timer ownership.
    fn timer_ownership(&self, actor: &ActorId) -> Option<ClientTimerOwnership>;
    /// Optional per-actor speed multiplier.
    fn speed_multiplier(&self, actor: &ActorId) -> Option<f32>;
    /// Current frame.
    fn frame(&self) -> ClientThinkFrame;
    /// Current settings.
    fn settings(&self) -> ClientThinkSettings;
    /// Clamp the pmove step cvar.
    fn set_pmove_msec(&self, ms: i32);
    /// Intermission think.
    fn intermission_think(&self, client: &ClientRef);
    /// Spectator think.
    fn spectator_think(&self, entity: &EntityRef, command: &UserCommand);
    /// Inactivity check.
    fn check_inactivity(&self, client: &ClientRef) -> bool;
    /// Free a grapple hook.
    fn free_hook(&self, hook: &EntityRef);
    /// Gauntlet attack check.
    fn check_gauntlet_attack(&self, entity: &EntityRef) -> bool;
    /// Client events dispatch.
    fn client_events(&self, entity: &EntityRef, old_sequence: i32);
    /// Respawn an entity.
    fn respawn(&self, entity: &EntityRef);
    /// Queue a console command.
    fn append_console_command(&self, command: &str);
    /// Whether an entity is a door trigger.
    fn is_door_trigger(&self, entity: &EntityRef) -> bool;
    /// Bot AAS test hook.
    fn bot_test_aas(&self, origin: Vec3);
}

pub(crate) const MASK_PLAYERSOLID: i32 = 1 | 0x10000 | 0x2000000;

pub(crate) const THINK_CONTENTS_BODY: i32 = 0x2000000;

pub(crate) const CONTENTS_BOTCLIP: i32 = 0x400000;

pub(crate) const CONTENTS_TRIGGER: i32 = 0x40000000;

pub(crate) const THINK_EF_FIRING: i32 = 0x100;

pub(crate) const REWARD_FLAGS: i32 = 0x8 | 0x40 | 0x800 | 0x8000 | 0x10000 | 0x20000;

pub(crate) fn think_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("ClientThink requires a game client"),
    }
}

pub(crate) fn bounds_overlap(first: &Bounds, second: &Bounds) -> bool {
    first.min.x <= second.max.x
        && first.min.y <= second.max.y
        && first.min.z <= second.max.z
        && first.max.x >= second.min.x
        && first.max.y >= second.min.y
        && first.max.z >= second.min.z
}

/// Authoritative command-path runtime (`ClientThinkRuntime`).
pub struct ClientThinkRuntime {
    /// Host services.
    pub host: Rc<dyn ClientThinkHost>,
}

impl ClientThinkRuntime {
    /// Fresh runtime.
    pub fn new(host: Rc<dyn ClientThinkHost>) -> Self {
        Self { host }
    }

    /// Receive a client command (`clientThink`).
    pub fn client_think(&self, client_num: usize, command: &UserCommand) {
        let entity = self.host.pool().at(client_num);
        let client = think_client_of(&entity);
        client.borrow_mut().pers.cmd = *command;
        client.borrow_mut().last_cmd_time = self.host.frame().time;
        let bot = entity.borrow().r.sv_flags & ServerEntityFlags::Bot as i32 != 0;
        if !bot && !self.host.settings().synchronous_clients {
            self.client_think_real(&entity);
        }
    }

    /// Run a bot/synchronous client (`runClient`).
    pub fn run_client(&self, entity: &EntityRef) {
        let bot = entity.borrow().r.sv_flags & ServerEntityFlags::Bot as i32 != 0;
        if !bot && !self.host.settings().synchronous_clients {
            return;
        }
        let client = think_client_of(entity);
        client.borrow_mut().pers.cmd.server_time = self.host.frame().time;
        self.client_think_real(entity);
    }

    /// Dispatch move contacts (`clientImpacts`).
    pub fn client_impacts(&self, entity: &EntityRef, contacts: &[ActorId]) {
        let this = entity.borrow().actor.clone();
        let mut seen: Vec<ActorId> = Vec::new();
        for actor in contacts {
            if seen.iter().any(|previous| previous == actor) {
                continue;
            }
            seen.push(actor.clone());
            let (bot, touched) = {
                let body = entity.borrow();
                (
                    body.r.sv_flags & ServerEntityFlags::Bot as i32 != 0,
                    body.touch.is_some(),
                )
            };
            if bot && touched {
                self.host.touches().touch(&this, actor);
            }
            self.host.touches().touch(actor, &this);
        }
    }

    /// Touch overlapping triggers (`touchTriggers`).
    pub fn touch_triggers(&self, entity: &EntityRef) {
        let this = entity.borrow().actor.clone();
        let client = match entity.borrow().client.clone() {
            Some(client) => client,
            None => return,
        };
        if client.borrow().ps.health() <= 0 {
            return;
        }
        let origin = client.borrow().ps.origin;
        let range = vec3(40.0, 40.0, 52.0);
        let touches = self.host.world().area_actors(
            &Bounds {
                min: sub3(origin, range),
                max: add3(origin, range),
            },
            MAX_GENTITIES,
        );
        let bounds = {
            let body = entity.borrow();
            Bounds {
                min: add3(origin, body.r.mins),
                max: add3(origin, body.r.maxs),
            }
        };
        for actor in &touches {
            let hit = self.host.touches().native(actor);
            let entity_touched = entity.borrow().touch.is_some();
            if let Some(hit) = &hit {
                if hit.borrow().touch.is_none() && !entity_touched {
                    continue;
                }
            }
            let trigger = match &hit {
                None => self.host.touches().is_trigger(actor),
                Some(hit) => hit.borrow().r.contents & CONTENTS_TRIGGER != 0,
            };
            if !trigger {
                continue;
            }
            if client.borrow().sess.session_team == Team::TeamSpectator as i32 {
                let allowed = match &hit {
                    Some(hit) => {
                        hit.borrow().s.e_type == EntityType::EtTeleportTrigger as i32 || self.host.is_door_trigger(hit)
                    }
                    None => false,
                };
                if !allowed {
                    continue;
                }
            }
            match &hit {
                Some(hit) if hit.borrow().s.e_type == EntityType::EtItem as i32 => {
                    let pos = hit.borrow().s.pos;
                    let touch = ItemsTrajectory {
                        trajectory_type: pos.trajectory_type as i32,
                        time: pos.time,
                        duration: pos.duration,
                        base: pos.base,
                        delta: pos.delta,
                    };
                    if !player_touches_item(origin, &touch, self.host.frame().time).unwrap() {
                        continue;
                    }
                }
                _ => {
                    if !self.host.world().contact_actor(&bounds, actor) {
                        continue;
                    }
                }
            }
            self.host.touches().touch(actor, &this);
            let bot = entity.borrow().r.sv_flags & ServerEntityFlags::Bot as i32 != 0;
            if bot && entity.borrow().touch.is_some() {
                self.host.touches().touch(&this, actor);
            }
        }
        let (jump_frame, pmove_frame) = {
            let record = client.borrow();
            (record.ps.jumppad_frame, record.ps.pmove_framecount)
        };
        if jump_frame != pmove_frame {
            let mut record = client.borrow_mut();
            record.ps.jumppad_frame = 0;
            record.ps.jumppad_ent = 0;
        }
    }

    /// Run one client think (`clientThinkReal`).
    pub fn client_think_real(&self, entity: &EntityRef) {
        let client = think_client_of(entity);
        if client.borrow().pers.connected != ConnectionState::Connected as i32 {
            return;
        }
        let frame = self.host.frame();
        let settings = self.host.settings();
        {
            let mut record = client.borrow_mut();
            if record.pers.cmd.server_time > frame.time.wrapping_add(200) {
                record.pers.cmd.server_time = frame.time.wrapping_add(200);
            }
            if record.pers.cmd.server_time < frame.time.wrapping_sub(1000) {
                record.pers.cmd.server_time = frame.time.wrapping_sub(1000);
            }
        }
        let mut msec = client
            .borrow()
            .pers
            .cmd
            .server_time
            .wrapping_sub(client.borrow().ps.command_time);
        if msec < 1 && client.borrow().sess.spectator_state != SpectatorState::Follow as i32 {
            return;
        }
        if msec > 200 {
            msec = 200;
        }
        if settings.pmove_msec < 8 {
            self.host.set_pmove_msec(8);
        } else if settings.pmove_msec > 33 {
            self.host.set_pmove_msec(33);
        }
        let fixed = settings.pmove_fixed || client.borrow().pers.pmove_fixed;
        if fixed {
            let mut record = client.borrow_mut();
            if settings.pmove_msec == 0 {
                // The source's imul collapses the infinite quotient to zero.
                record.pers.cmd.server_time = 0;
            } else {
                let rounded = record
                    .pers
                    .cmd
                    .server_time
                    .wrapping_add(settings.pmove_msec)
                    .wrapping_sub(1)
                    / settings.pmove_msec;
                record.pers.cmd.server_time = rounded.wrapping_mul(settings.pmove_msec);
            }
        }
        if frame.intermission_time != 0 {
            self.host.intermission_think(&client);
            return;
        }
        if client.borrow().sess.session_team == Team::TeamSpectator as i32 {
            if client.borrow().sess.spectator_state != SpectatorState::Scoreboard as i32 {
                let command = client.borrow().pers.cmd;
                self.host.spectator_think(entity, &command);
            }
            return;
        }
        if !self.host.check_inactivity(&client) {
            return;
        }
        if frame.time > client.borrow().reward_time {
            client.borrow_mut().ps.e_flags &= !REWARD_FLAGS;
        }
        {
            let mut record = client.borrow_mut();
            record.ps.pm_type = if record.noclip {
                MoveType::PmNoclip as i32
            } else if record.ps.health() <= 0 {
                MoveType::PmDead as i32
            } else {
                MoveType::PmNormal as i32
            };
            record.ps.gravity = settings.gravity.trunc() as i32;
            record.ps.speed = settings.speed.trunc() as i32;
        }
        let actor = entity.borrow().actor.clone();
        let speed_multiplier = self
            .host
            .speed_multiplier(&actor)
            .unwrap_or_else(|| client_speed_multiplier(self.host.items().as_ref(), &client.borrow().ps));
        if speed_multiplier != 1.0 {
            let mut record = client.borrow_mut();
            record.ps.speed = (record.ps.speed as f32 * speed_multiplier).trunc() as i32;
        }
        let (weapon, hook) = {
            let record = client.borrow();
            (record.ps.weapon, record.hook.clone())
        };
        let buttons = client.borrow().pers.cmd.buttons;
        if weapon == Weapon::WpGrapplingHook as i32 && hook.is_some() && buttons & CommandButtons::Attack as i32 == 0 {
            if let Some(hook) = hook {
                self.host.free_hook(&hook);
            }
        }
        let old_event_sequence = client.borrow().ps.event_sequence;
        let (weapon_time, weapon_now) = {
            let record = client.borrow();
            (record.ps.weapon_time, record.ps.weapon)
        };
        let gauntlet_hit = weapon_now == Weapon::WpGauntlet as i32
            && buttons & CommandButtons::Talk as i32 == 0
            && buttons & CommandButtons::Attack as i32 != 0
            && weapon_time <= 0
            && self.host.check_gauntlet_attack(entity);
        if entity.borrow().flags & GameFlags::FORCE_GESTURE != 0 {
            entity.borrow_mut().flags &= !GameFlags::FORCE_GESTURE;
            client.borrow_mut().pers.cmd.buttons |= CommandButtons::Gesture as i32;
        }
        expand_q3_invulnerability(&self.host.pool(), self.host.world().as_ref(), entity);
        let trace_mask = {
            let body = entity.borrow();
            if client.borrow().ps.pm_type == MoveType::PmDead as i32 {
                MASK_PLAYERSOLID & !THINK_CONTENTS_BODY
            } else if body.r.sv_flags & ServerEntityFlags::Bot as i32 != 0 {
                MASK_PLAYERSOLID | CONTENTS_BOTCLIP
            } else {
                MASK_PLAYERSOLID
            }
        };
        let origin = client.borrow().ps.origin;
        client.borrow_mut().old_origin = origin;
        let mut movement_command = client.borrow().pers.cmd;
        if client.borrow().ps.product == Product::Missionpack
            && frame.intermission_queued != 0
            && settings.single_player
        {
            let elapsed = frame.time.wrapping_sub(frame.intermission_queued);
            if elapsed >= 1000 {
                movement_command.buttons = 0;
                movement_command.forwardmove = 0;
                movement_command.rightmove = 0;
                movement_command.upmove = 0;
                if (2000..=2500).contains(&elapsed) {
                    self.host.append_console_command("centerview\n");
                }
                client.borrow_mut().ps.pm_type = MoveType::PmSpintermission as i32;
            }
        }
        let movement = self.host.move_client(
            entity,
            &movement_command,
            &ClientMovementOptions {
                debug_level: settings.debug_move,
                trace_mask,
                fixed_msec: if fixed { Some(settings.pmove_msec) } else { None },
                gauntlet_hit,
                no_footsteps: settings.dmflags & 32 != 0,
            },
        );
        if client.borrow().ps.event_sequence != old_event_sequence {
            entity.borrow_mut().event_time = frame.time;
        }
        {
            let mut record = client.borrow_mut();
            let mut body = entity.borrow_mut();
            if settings.smooth_clients {
                let command_time = record.ps.command_time;
                player_state_to_entity_state_extrapolate(&mut record.ps, &mut body.s, command_time, true);
            } else {
                player_state_to_entity_state(&mut record.ps, &mut body.s, true);
            }
        }
        send_pending_predictable_events(self.host.effects().as_ref(), &mut client.borrow_mut().ps);
        if client.borrow().ps.e_flags & THINK_EF_FIRING == 0 {
            client.borrow_mut().fire_held = false;
        }
        entity.borrow_mut().r.mins = movement.bounds.min;
        entity.borrow_mut().r.maxs = movement.bounds.max;
        entity.borrow_mut().waterlevel = movement.waterlevel;
        entity.borrow_mut().watertype = movement.watertype;
        let base = entity.borrow().s.pos.base;
        let noclip = client.borrow().noclip;
        with_entity_origin(entity, base, || {
            self.host.client_events(entity, old_event_sequence);
            self.host.world().link(entity);
            if !noclip {
                self.touch_triggers(entity);
            }
        });
        entity.borrow_mut().r.set_current_origin(client.borrow().ps.origin);
        self.host.bot_test_aas(entity.borrow().r.current_origin());
        self.client_impacts(entity, &movement.contacts);
        if client.borrow().ps.event_sequence != old_event_sequence {
            entity.borrow_mut().event_time = frame.time;
        }
        {
            let mut record = client.borrow_mut();
            record.old_buttons = record.buttons;
            record.buttons = record.pers.cmd.buttons;
            record.latched_buttons |= record.buttons & !record.old_buttons;
        }
        if client.borrow().ps.health() <= 0 {
            if frame.time > client.borrow().respawn_time {
                let since = frame.time.wrapping_sub(client.borrow().respawn_time);
                if settings.force_respawn_seconds > 0 && since > settings.force_respawn_seconds.wrapping_mul(1000) {
                    self.host.respawn(entity);
                    return;
                }
                if client.borrow().pers.cmd.buttons
                    & (CommandButtons::Attack as i32 | CommandButtons::UseHoldable as i32)
                    != 0
                {
                    self.host.respawn(entity);
                }
            }
            return;
        }
        let ownership = self.host.timer_ownership(&actor);
        client_timer_actions(self.host.effects().as_ref(), entity, msec, ownership.as_ref());
    }
}

pub(crate) fn stuck_in_other_client(pool: &EntityPool, world: &dyn Q3World, entity: &EntityRef) -> bool {
    let zero = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(0.0, 0.0, 0.0),
    };
    let slot = entity.borrow().slot;
    let bounds = world
        .link_state(slot as i32)
        .map(|state| state.absbounds)
        .unwrap_or(zero);
    for index in 0..MAX_CLIENTS {
        let other = pool.at(index);
        if Rc::ptr_eq(&other, entity) {
            continue;
        }
        let body = other.borrow();
        if !body.inuse || body.client.is_none() || body.health <= 0 {
            continue;
        }
        drop(body);
        let other_bounds = world
            .link_state(index as i32)
            .map(|state| state.absbounds)
            .unwrap_or(zero);
        if bounds_overlap(&bounds, &other_bounds) {
            return true;
        }
    }
    false
}

/// Expand invulnerability bounds when clear (`expandQ3Invulnerability`).
pub fn expand_q3_invulnerability(pool: &EntityPool, world: &dyn Q3World, entity: &EntityRef) {
    let client = think_client_of(entity);
    let expand = {
        let record = client.borrow();
        record.ps.product == Product::Missionpack
            && record.ps.powerups.get(Powerup::PwInvulnerability as usize) != 0
            && record.ps.pm_flags & MoveFlags::InvulExpand as i32 == 0
    };
    if !expand {
        return;
    }
    let (old_mins, old_maxs) = {
        let body = entity.borrow();
        (body.r.mins, body.r.maxs)
    };
    entity.borrow_mut().r.mins = vec3(-42.0, -42.0, -42.0);
    entity.borrow_mut().r.maxs = vec3(42.0, 42.0, 42.0);
    world.link(entity);
    if !stuck_in_other_client(pool, world, entity) {
        client.borrow_mut().ps.pm_flags |= MoveFlags::InvulExpand as i32;
    }
    entity.borrow_mut().r.mins = old_mins;
    entity.borrow_mut().r.maxs = old_maxs;
    world.link(entity);
}
