//! Quake III team-arena: client effects.
//!
//! Donor provenance: `src/content/q3/team-arena/client-effects.ts`.

use qa_core::math::vector_to_angles;
use std::cell::Cell;
use std::rc::Rc;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::game::combat::DamageFlags;
use crate::q3::base::game::state::GameFlags;
use crate::q3::base::shared::definitions::*;
use crate::q3::base::shared::entity_shared::ServerEntityFlags;
use crate::q3::team_arena::support::*;

// ---------------------------------------------------------------------------
// client-effects.ts
// ---------------------------------------------------------------------------

/// Core effect services (combat plus item lookups).
pub trait EffectsCore {
    /// Combat services.
    fn combat(&self) -> CombatRef;
    /// Item services.
    fn items(&self) -> Rc<dyn ItemHost>;
}

/// Shared core-effects handle.
pub type EffectsCoreRef = Rc<dyn EffectsCore>;

/// Client effect services (`ClientEffectsContext`).
pub trait EffectsHost: EffectsCore {
    /// Intermission time.
    fn intermission_time(&self) -> i32;
    /// Whether clients are smoothed.
    fn smooth_clients(&self) -> bool;
    /// Fry loop sound.
    fn fry_sound(&self) -> i32;
    /// Random integer.
    fn random_int(&self) -> i32;
    /// Sound index for a path.
    fn sound_index(&self, path: &str) -> i32;
    /// Play an entity sound.
    fn sound(&self, entity: &EntityRef, channel: i32, sound_index: i32);
    /// Spectator end-of-frame.
    fn spectator_end_frame(&self, entity: &EntityRef);
}

/// Shared effects handle.
pub type EffectsRef = Rc<dyn EffectsHost>;

pub(crate) const CONTENTS_LAVA: i32 = 8;

pub(crate) const CONTENTS_SLIME: i32 = 16;

pub(crate) const EFFECTS_EF_TICKING: i32 = 2;

pub(crate) const EFFECTS_EF_CONNECTION: i32 = 0x2000;

pub(crate) const EFFECTS_EF_PLAYER_EVENT: i32 = 0x10;

pub(crate) const CHAN_VOICE: i32 = 3;

pub(crate) fn effects_client_of(entity: &EntityRef) -> ClientRef {
    match entity.borrow().client.clone() {
        Some(client) => client,
        None => panic!("Client effects require a client entity"),
    }
}

/// Publish damage feedback (`damageFeedback`).
pub fn damage_feedback<C: EffectsCore + ?Sized>(context: &C, player: &EntityRef) {
    let client = effects_client_of(player);
    if client.borrow().ps.pm_type == MoveType::PmDead as i32 {
        return;
    }
    let (blood, armor) = {
        let record = client.borrow();
        (record.damage_blood, record.damage_armor)
    };
    let count = 255i32.min(blood.wrapping_add(armor));
    if count == 0 {
        return;
    }
    if client.borrow().damage_from_world {
        let mut record = client.borrow_mut();
        record.ps.damage_pitch = 255;
        record.ps.damage_yaw = 255;
        record.damage_from_world = false;
    } else {
        let from = client.borrow().damage_from;
        let angles = vector_to_angles(from);
        let mut record = client.borrow_mut();
        record.ps.damage_pitch = ((angles.x / 360.0 * 256.0).trunc()) as i32;
        record.ps.damage_yaw = ((angles.y / 360.0 * 256.0).trunc()) as i32;
    }
    let time = context.combat().time();
    let pain_at = player.borrow().pain_debounce_time;
    let flags = player.borrow().flags;
    if time > pain_at && flags & GameFlags::GODMODE == 0 {
        player.borrow_mut().pain_debounce_time = time.wrapping_add(700);
        let health = player.borrow().health;
        context
            .combat()
            .pool()
            .add_event(player, EntityEvent::EvPain as i32, health);
        let mut record = client.borrow_mut();
        record.ps.damage_event = record.ps.damage_event.wrapping_add(1);
    }
    let mut record = client.borrow_mut();
    record.ps.damage_count = count;
    record.damage_blood = 0;
    record.damage_armor = 0;
    record.damage_knockback = 0;
}

/// Drowning and lava/slime damage (`worldEffects`).
pub fn world_effects(context: &dyn EffectsHost, entity: &EntityRef) {
    let client = effects_client_of(entity);
    let time = context.combat().time();
    if client.borrow().noclip {
        client.borrow_mut().air_out_time = time.wrapping_add(12_000);
        return;
    }
    let waterlevel = entity.borrow().waterlevel;
    let suit = {
        let record = client.borrow();
        record.ps.powerups.get(Powerup::PwBattlesuit as usize) > time
    };
    if waterlevel == 3 {
        if suit {
            client.borrow_mut().air_out_time = time.wrapping_add(10_000);
        }
        if client.borrow().air_out_time < time {
            {
                let mut record = client.borrow_mut();
                record.air_out_time = record.air_out_time.wrapping_add(1000);
            }
            if entity.borrow().health > 0 {
                {
                    let mut body = entity.borrow_mut();
                    body.damage = 15.min(body.damage.wrapping_add(2));
                }
                let (health, damage) = {
                    let body = entity.borrow();
                    (body.health, body.damage)
                };
                let path = if health <= damage {
                    "*drown.wav"
                } else if context.random_int() & 1 != 0 {
                    "sound/player/gurp1.wav"
                } else {
                    "sound/player/gurp2.wav"
                };
                let index = context.sound_index(path);
                context.sound(entity, CHAN_VOICE, index);
                entity.borrow_mut().pain_debounce_time = time.wrapping_add(200);
                context
                    .combat()
                    .damage(entity, None, None, None, None, damage, DamageFlags::NO_ARMOR, 14);
            }
        }
    } else {
        client.borrow_mut().air_out_time = time.wrapping_add(12_000);
        entity.borrow_mut().damage = 2;
    }
    let (watertype, health, pain_at) = {
        let body = entity.borrow();
        (body.watertype, body.health, body.pain_debounce_time)
    };
    if waterlevel != 0 && watertype & (CONTENTS_LAVA | CONTENTS_SLIME) != 0 && health > 0 && pain_at <= time {
        if suit {
            context
                .combat()
                .pool()
                .add_event(entity, EntityEvent::EvPowerupBattlesuit as i32, 0);
        } else {
            if watertype & CONTENTS_LAVA != 0 {
                context
                    .combat()
                    .damage(entity, None, None, None, None, 30i32.wrapping_mul(waterlevel), 0, 16);
            }
            if watertype & CONTENTS_SLIME != 0 {
                context
                    .combat()
                    .damage(entity, None, None, None, None, 10i32.wrapping_mul(waterlevel), 0, 15);
            }
        }
    }
}

/// Select the client loop sound (`setClientSound`).
pub fn set_client_sound(context: &dyn EffectsHost, entity: &EntityRef) {
    let client = effects_client_of(entity);
    let ticking =
        context.combat().product() == Product::Missionpack && entity.borrow().s.e_flags & EFFECTS_EF_TICKING != 0;
    let (waterlevel, watertype) = {
        let body = entity.borrow();
        (body.waterlevel, body.watertype)
    };
    let loop_sound = if ticking {
        context.sound_index("sound/weapons/proxmine/wstbtick.wav")
    } else if waterlevel != 0 && watertype & (CONTENTS_LAVA | CONTENTS_SLIME) != 0 {
        context.fry_sound()
    } else {
        0
    };
    client.borrow_mut().ps.loop_sound = loop_sound;
}

/// Ammo-regeneration rule (`Q3AmmoRegenerationRule`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3AmmoRegenerationRule {
    /// Weapon tag.
    pub weapon: i32,
    /// Maximum ammo.
    pub max: i32,
    /// Increment per tick.
    pub increment: i32,
    /// Milliseconds per tick.
    pub time: i32,
}

pub(crate) const AMMO_REGENERATION: &[Q3AmmoRegenerationRule] = &[
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpMachinegun as i32,
        max: 50,
        increment: 4,
        time: 1000,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpShotgun as i32,
        max: 10,
        increment: 1,
        time: 1500,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpGrenadeLauncher as i32,
        max: 10,
        increment: 1,
        time: 2000,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpRocketLauncher as i32,
        max: 10,
        increment: 1,
        time: 1750,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpLightning as i32,
        max: 50,
        increment: 5,
        time: 1500,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpRailgun as i32,
        max: 10,
        increment: 1,
        time: 1750,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpPlasmagun as i32,
        max: 50,
        increment: 5,
        time: 1500,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpBfg as i32,
        max: 10,
        increment: 1,
        time: 4000,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpNailgun as i32,
        max: 10,
        increment: 1,
        time: 1250,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpProxLauncher as i32,
        max: 5,
        increment: 1,
        time: 2000,
    },
    Q3AmmoRegenerationRule {
        weapon: Weapon::WpChaingun as i32,
        max: 100,
        increment: 5,
        time: 1000,
    },
];

pub(crate) fn persistent_tag(items: &dyn ItemHost, client: &ClientRef) -> i32 {
    let record = client.borrow();
    match stat_schema(record.ps.product) {
        StatSchema::Base(_) => Powerup::PwNone as i32,
        StatSchema::Missionpack(layout) => {
            let slot = layout.persistent_powerup as usize;
            items.item_at(record.ps.product, record.ps.stats.get(slot) as usize).tag
        }
    }
}

/// Scout/haste speed multiplier (`clientSpeedMultiplier`).
#[must_use]
pub fn client_speed_multiplier(items: &dyn ItemHost, ps: &PlayerState) -> f32 {
    if let StatSchema::Missionpack(layout) = stat_schema(ps.product) {
        let tag = items
            .item_at(ps.product, ps.stats.get(layout.persistent_powerup as usize) as usize)
            .tag;
        if tag == Powerup::PwScout as i32 {
            return 1.5;
        }
    }
    if ps.powerups.get(Powerup::PwHaste as usize) != 0 {
        1.3
    } else {
        1.0
    }
}

/// Ammo-regeneration rule for a weapon (`q3AmmoRegenerationRule`).
#[must_use]
pub fn q3_ammo_regeneration_rule(weapon_tag: i32) -> Q3AmmoRegenerationRule {
    match AMMO_REGENERATION.iter().find(|rule| rule.weapon == weapon_tag) {
        Some(rule) => *rule,
        None => panic!("Weapon has no original Ammo Regen rule"),
    }
}

/// Ammo-regeneration step (`stepQ3AmmoRegeneration`).
#[must_use]
pub fn step_q3_ammo_regeneration(
    rule: &Q3AmmoRegenerationRule,
    count: i32,
    milliseconds: i32,
    msec: i32,
) -> (i32, Option<i32>) {
    let mut elapsed = milliseconds.wrapping_add(msec);
    if count >= rule.max {
        elapsed = 0;
    }
    if elapsed < rule.time {
        return (elapsed, None);
    }
    while elapsed >= rule.time {
        elapsed -= rule.time;
    }
    (elapsed, Some(rule.max.min(count.wrapping_add(rule.increment))))
}

/// Externally owned ammo timer (`Q3MappedAmmoTimer`).
pub struct Q3MappedAmmoTimer {
    /// Rule.
    pub rule: Q3AmmoRegenerationRule,
    /// Whether the mapping is still current.
    pub current: Rc<dyn Fn() -> bool>,
    /// Ammo count.
    pub count: Cell<i32>,
    /// Elapsed milliseconds.
    pub elapsed_ms: Cell<i32>,
}

/// Timer ownership (`ClientTimerOwnership`).
pub struct ClientTimerOwnership {
    /// Whether ordinary decay applies.
    pub ordinary_decay: bool,
    /// Externally owned ammo timers.
    pub ammo: Option<Vec<Q3MappedAmmoTimer>>,
}

impl ClientTimerOwnership {
    /// Native ownership.
    pub fn native() -> Self {
        Self {
            ordinary_decay: true,
            ammo: None,
        }
    }
}

/// Health/armor decay and ammo regeneration (`clientTimerActions`).
pub fn client_timer_actions<C: EffectsCore + ?Sized>(
    context: &C,
    entity: &EntityRef,
    msec: i32,
    ownership: Option<&ClientTimerOwnership>,
) {
    let native = ClientTimerOwnership::native();
    let ownership = ownership.unwrap_or(&native);
    let client = effects_client_of(entity);
    let residual = client.borrow().time_residual;
    client.borrow_mut().time_residual = residual.wrapping_add(msec);
    while client.borrow().time_residual >= 1000 {
        client.borrow_mut().time_residual -= 1000;
        let (product, maximum, regen, armor) = {
            let record = client.borrow();
            let (max_health_slot, armor_slot) = match stat_schema(record.ps.product) {
                StatSchema::Base(layout) => (layout.max_health, layout.armor),
                StatSchema::Missionpack(layout) => (layout.max_health, layout.armor),
            };
            (
                record.ps.product,
                record.ps.stats.get(max_health_slot as usize),
                record.ps.powerups.get(Powerup::PwRegen as usize),
                record.ps.stats.get(armor_slot as usize),
            )
        };
        let tag = persistent_tag(context.items().as_ref(), &client);
        let max_health = if product == Product::Missionpack && tag == Powerup::PwGuard as i32 {
            maximum / 2
        } else if regen != 0 {
            maximum
        } else {
            0
        };
        if (product == Product::Baseq3 && regen != 0) || max_health != 0 {
            let health = entity.borrow().health;
            if health < max_health {
                let grown = health.wrapping_add(15);
                let cap = max_health as f32 * 1.1;
                entity.borrow_mut().health = if grown as f32 > cap { cap.trunc() as i32 } else { grown };
                context
                    .combat()
                    .pool()
                    .add_event(entity, EntityEvent::EvPowerupRegen as i32, 0);
            } else if health < max_health.wrapping_mul(2) {
                let grown = health.wrapping_add(5);
                let cap = max_health.wrapping_mul(2);
                entity.borrow_mut().health = if grown > cap { cap } else { grown };
                context
                    .combat()
                    .pool()
                    .add_event(entity, EntityEvent::EvPowerupRegen as i32, 0);
            }
        } else if ownership.ordinary_decay && entity.borrow().health > maximum {
            let health = entity.borrow().health;
            entity.borrow_mut().health = health.wrapping_sub(1);
        }
        if ownership.ordinary_decay && armor > maximum {
            let armor_slot = match stat_schema(product) {
                StatSchema::Base(layout) => layout.armor,
                StatSchema::Missionpack(layout) => layout.armor,
            };
            client.borrow_mut().ps.stats.set(armor_slot as usize, armor - 1);
        }
    }
    let product = client.borrow().ps.product;
    if product == Product::Missionpack
        && persistent_tag(context.items().as_ref(), &client) == Powerup::PwAmmoregen as i32
    {
        match &ownership.ammo {
            None => {
                for rule in AMMO_REGENERATION {
                    let (count, elapsed) = {
                        let record = client.borrow();
                        (
                            record.ps.ammo.get(rule.weapon as usize),
                            record.ammo_times.get(rule.weapon as usize),
                        )
                    };
                    let (next_elapsed, next_count) = step_q3_ammo_regeneration(rule, count, elapsed, msec);
                    if let Some(next_count) = next_count {
                        client.borrow_mut().ps.ammo.set(rule.weapon as usize, next_count);
                    }
                    client.borrow_mut().ammo_times.set(rule.weapon as usize, next_elapsed);
                }
            }
            Some(timers) => {
                for timer in timers {
                    if !(timer.current)() {
                        break;
                    }
                    let (next_elapsed, next_count) =
                        step_q3_ammo_regeneration(&timer.rule, timer.count.get(), timer.elapsed_ms.get(), msec);
                    if let Some(next_count) = next_count {
                        timer.count.set(next_count);
                    }
                    if (timer.current)() {
                        timer.elapsed_ms.set(next_elapsed);
                    }
                }
            }
        }
    }
}

/// Broadcast one pending predictable event (`sendPendingPredictableEvents`).
pub fn send_pending_predictable_events<C: EffectsCore + ?Sized>(context: &C, ps: &mut PlayerState) {
    if ps.entity_event_sequence >= ps.event_sequence {
        return;
    }
    let event = ps.events.get((ps.entity_event_sequence & 1) as usize) | ((ps.entity_event_sequence & 3) << 8);
    let external = ps.external_event;
    ps.external_event = 0;
    let temporary = context.combat().pool().temp_entity(ps.origin, event);
    let number = temporary.borrow().slot as i32;
    {
        let mut body = temporary.borrow_mut();
        player_state_to_entity_state(ps, &mut body.s, true);
        body.s.number = number;
        body.s.e_type = EntityType::EtEvents as i32 + event;
        body.s.e_flags |= EFFECTS_EF_PLAYER_EVENT;
        body.s.other_entity_num = ps.client_num;
        body.r.sv_flags |= ServerEntityFlags::Notsingleclient as i32;
        body.r.single_client = ps.client_num;
    }
    ps.external_event = external;
}

/// Expire powerups and pin persistent ones (`updateQ3ClientPowerups`).
pub fn update_q3_client_powerups<C: EffectsCore + ?Sized>(context: &C, client: &ClientRef) {
    let time = context.combat().time();
    {
        let record = client.borrow();
        for index in 0..record.ps.powerups.len() {
            if record.ps.powerups.get(index) < time {
                record.ps.powerups.set(index, 0);
            }
        }
    }
    if context.combat().product() == Product::Missionpack {
        let tag = persistent_tag(context.items().as_ref(), client);
        for powerup_tag in [
            Powerup::PwGuard as i32,
            Powerup::PwScout as i32,
            Powerup::PwDoubler as i32,
            Powerup::PwAmmoregen as i32,
        ] {
            if tag == powerup_tag {
                client.borrow_mut().ps.powerups.set(powerup_tag as usize, time);
            }
        }
        if client.borrow().invulnerability_time > time {
            client
                .borrow_mut()
                .ps
                .powerups
                .set(Powerup::PwInvulnerability as usize, time);
        }
    }
}

/// Finish a client frame (`clientEndFrame`).
pub fn client_end_frame(context: &dyn EffectsHost, entity: &EntityRef) {
    let client = effects_client_of(entity);
    if client.borrow().sess.session_team == Team::TeamSpectator as i32 {
        context.spectator_end_frame(entity);
        return;
    }
    let time = context.combat().time();
    update_q3_client_powerups(context, &client);
    if context.intermission_time() != 0 {
        return;
    }
    world_effects(context, entity);
    damage_feedback(context, entity);
    let lagged = time.wrapping_sub(client.borrow().last_cmd_time) > 1000;
    {
        let mut body = entity.borrow_mut();
        if lagged {
            body.s.e_flags |= EFFECTS_EF_CONNECTION;
        } else {
            body.s.e_flags &= !EFFECTS_EF_CONNECTION;
        }
    }
    let health = entity.borrow().health;
    client.borrow_mut().ps.set_health(health);
    set_client_sound(context, entity);
    {
        let mut record = client.borrow_mut();
        let mut body = entity.borrow_mut();
        if context.smooth_clients() {
            let command_time = record.ps.command_time;
            player_state_to_entity_state_extrapolate(&mut record.ps, &mut body.s, command_time, true);
        } else {
            player_state_to_entity_state(&mut record.ps, &mut body.s, true);
        }
    }
    send_pending_predictable_events(context, &mut client.borrow_mut().ps);
}
