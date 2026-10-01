//! Q2 rerelease view (`src/content/q2/rerelease/view.ts`).
//!
//! Gameplay logic adapted from id Software's Quake II game and the
//! rerelease game DLL (GPL-2.0-or-later).

use qa_core::identity::ActorId;
use qa_core::math::{dot3, normalize3, sub3, vec3, Vec3};

use crate::q2::base::player::index::player_hooks;
use crate::q2::base::player::types::{Q2BodyChanges, Q2CharacterContext, Q2PlayerContext, Q2PlayerView};
use crate::q2::base::player::view::{q2_build_view, q2_damage_feedback};
use crate::q2::foundation::host::{Q2Edition, Q2GameServices, Q2Mode, Q2PresentationEvent};
use crate::q2::foundation::weapons::types::Q2WeaponPhase;
use crate::q2::foundation::weapons::vectors::angle_vectors;
use crate::q2::support::contracts::BodyState;

/// Clamp a scalar.
fn clamp(value: f64, low: f64, high: f64) -> f64 {
    value.max(low).min(high)
}

/// Kick decay ratio (`kickRatio`).
fn kick_ratio(until: f64, now: f64, duration: f64, slack: f64) -> f64 {
    let remaining = until - now;
    if remaining <= 0.0 {
        return 0.0;
    }
    if slack != 0.0 && remaining > duration {
        (duration + slack - remaining) / slack
    } else {
        remaining / duration
    }
}

/// Rerelease damage feedback (`q2RereleaseDamageFeedback`).
pub fn q2_rerelease_damage_feedback(actor: ActorId, game: &mut Q2GameServices, pain_index: i32) -> (i32, i32) {
    let mut context = Q2PlayerContext {
        actor: actor.clone(),
        game,
    };
    let now = context.now();
    let snapshot = context.state_snapshot();
    let (blood, armor, power, kick) = (
        snapshot.damage_blood,
        snapshot.damage_armor,
        snapshot.damage_power_armor,
        snapshot.damage_knockback,
    );
    let total = blood + armor + power;
    let (old_alpha, old_pain) = (snapshot.damage_alpha, snapshot.pain_debounce);
    let (flashes, pain_index) = q2_damage_feedback(&mut context, pain_index);
    let view_angles = context.movement().view_angles;
    let game = &mut *context.game;
    {
        let extra = game
            .rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease state is missing");
        if flashes != 0 {
            extra.flashes = flashes;
            extra.flash_time = now + 0.1;
        } else if extra.flash_time < now {
            extra.flashes = 0;
        }
    }
    if total != 0.0 {
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease state is missing")
            .animation_time = 0.0;
        let count = if blood != 0.0 { total.max(10.0) } else { total.min(2.0) };
        {
            let state = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            state.damage_alpha = 0.0f64.max(old_alpha);
            if blood != 0.0 || state.damage_alpha + count * 0.06 < 0.15 {
                state.damage_alpha = clamp(state.damage_alpha + count * 0.06, 0.06, 0.4);
            }
            state.damage_blend = normalize3(vec3(
                (armor / total
                    + if blood != 0.0 {
                        (15.0f64).max(blood / total)
                    } else {
                        0.0
                    }) as f32,
                ((power + armor) / total) as f32,
                (armor / total) as f32,
            ));
        }
        let health = game
            .host
            .combat()
            .read(&actor)
            .map(|combat| combat.health)
            .unwrap_or(0.0);
        if kick != 0.0 && health > 0.0 {
            let amount = clamp(kick.abs() * 100.0 / health, count * 0.5, 50.0);
            let origin = game.body_of(actor.clone()).origin;
            let from = game
                .players
                .states
                .get(&actor)
                .expect("Q2 player has not been admitted")
                .damage_from;
            let direction = normalize3(sub3(from, origin));
            let axes = angle_vectors(view_angles);
            let state = game
                .players
                .states
                .get_mut(&actor)
                .expect("Q2 player has not been admitted");
            state.damage_roll = amount * f64::from(dot3(direction, axes.right)) * 0.3;
            state.damage_pitch = -amount * f64::from(dot3(direction, axes.forward)) * 0.3;
            state.damage_time = now + 0.5 + (0.1 - game.host.frame_seconds());
        }
        let pained = game
            .players
            .states
            .get(&actor)
            .expect("Q2 player has not been admitted")
            .pain_debounce
            > old_pain;
        if pained {
            let origin = game.body_of(actor.clone()).origin;
            (player_hooks(game).noise)(actor.clone(), origin);
        }
    }
    let flashes = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease state is missing")
        .flashes;
    (flashes, pain_index)
}

/// View context with warning-sound filtering.
struct RereleaseViewContext<'game> {
    /// Inner player context.
    inner: Q2PlayerContext<'game>,
    /// Quad expiry.
    quad_until: f64,
    /// Invulnerability expiry.
    invulnerability_until: f64,
    /// Enviro expiry.
    enviro_until: f64,
    /// Breather expiry.
    breather_until: f64,
    /// Current time.
    now: f64,
}

impl Q2CharacterContext for RereleaseViewContext<'_> {
    fn actor_id(&self) -> ActorId {
        self.inner.actor_id()
    }
    fn owned_actor(&self) -> qa_core::identity::OwnedActor {
        self.inner.owned_actor()
    }
    fn now(&mut self) -> f64 {
        self.inner.now()
    }
    fn random(&mut self) -> f64 {
        self.inner.random()
    }
    fn movement(&mut self) -> crate::q2::base::player::types::Q2PlayerMovement {
        self.inner.movement()
    }
    fn rules(&self) -> crate::q2::base::player::types::Q2PlayerRules {
        self.inner.rules()
    }
    fn powerups(&mut self) -> crate::q2::foundation::items::Q2PlayerPowerups {
        self.inner.powerups()
    }
    fn weapon_state(&mut self) -> Option<crate::q2::base::player::types::Q2CharacterWeapon> {
        self.inner.weapon_state()
    }
    fn environment_damage(&mut self, amount: f64, means: i32, flags: i32) {
        self.inner.environment_damage(amount, means, flags);
    }
    fn noise(&mut self, origin: Vec3) {
        self.inner.noise(origin);
    }
    fn body(&mut self) -> BodyState {
        self.inner.body()
    }
    fn move_body(&mut self, changes: Q2BodyChanges, link: bool) {
        self.inner.move_body(changes, link);
    }
    fn sound(&mut self, path: &str, channel: i32, volume: f64, attenuation: f64) {
        let until = if path == "items/damage2.wav" {
            Some(self.quad_until)
        } else if path == "items/protect2.wav" {
            Some(self.invulnerability_until)
        } else if path == "items/airout.wav" {
            Some(if self.enviro_until > self.now {
                self.enviro_until
            } else {
                self.breather_until
            })
        } else {
            None
        };
        if let Some(until) = until {
            if ((until - self.now) * 1000.0).round() as i64 != 3000 {
                return;
            }
        }
        self.inner.sound(path, channel, volume, attenuation);
    }
    fn emit(&mut self, event: Q2PresentationEvent) {
        self.inner.emit(event);
    }
    fn point_contents(&mut self, point: Vec3) -> i32 {
        self.inner.point_contents(point)
    }
    fn combat(&mut self) -> Option<crate::q2::support::contracts::CombatState> {
        self.inner.combat()
    }
    fn inventory_count(&mut self, item: &crate::contract::ItemId) -> f64 {
        self.inner.inventory_count(item)
    }
    fn mode(&self) -> Q2Mode {
        self.inner.mode()
    }
    fn deathmatch_flags(&self) -> i32 {
        self.inner.deathmatch_flags()
    }
    fn edition(&self) -> Q2Edition {
        self.inner.edition()
    }
    fn state_snapshot(&mut self) -> crate::q2::base::player::types::Q2PlayerState {
        self.inner.state_snapshot()
    }
    fn with_state<R>(&mut self, f: impl FnOnce(&mut crate::q2::base::player::types::Q2PlayerState) -> R) -> R {
        self.inner.with_state(f)
    }
    fn entity_snapshot(&mut self) -> crate::q2::foundation::host::Q2Entity {
        self.inner.entity_snapshot()
    }
    fn with_entity<R>(&mut self, f: impl FnOnce(&mut crate::q2::foundation::host::Q2Entity) -> R) -> R {
        self.inner.with_entity(f)
    }
}

/// Rerelease view construction (`q2RereleaseBuildView`).
pub fn q2_rerelease_build_view(
    actor: ActorId,
    game: &mut Q2GameServices,
    flashes: i32,
    intermission: bool,
) -> Q2PlayerView {
    let mut context = Q2PlayerContext {
        actor: actor.clone(),
        game,
    };
    let now = context.now();
    let frame = context.game.host.frame_seconds();
    let powers = context.powerups();
    let mut shared = RereleaseViewContext {
        inner: context,
        quad_until: powers.quad_until,
        invulnerability_until: powers.invulnerability_until,
        enviro_until: powers.enviro_until,
        breather_until: powers.breather_until,
        now,
    };
    let snapshot = shared.inner.state_snapshot();
    let (alpha, bonus) = (snapshot.damage_alpha, snapshot.bonus_alpha);
    let base = q2_build_view(&mut shared, flashes, intermission);
    let game = &mut *shared.inner.game;
    {
        let state = game
            .players
            .states
            .get_mut(&actor)
            .expect("Q2 player has not been admitted");
        state.damage_alpha = 0.0f64.max(alpha - frame * 0.6);
        state.bonus_alpha = 0.0f64.max(bonus - frame);
    }
    if intermission {
        return Q2PlayerView {
            offset: vec3(0.0, 0.0, 0.0),
            ..base
        };
    }
    let body = game.body_of(actor.clone());
    let movement = (player_hooks(game).movement)(actor.clone());
    let rules = game.players.rules.clone();
    let source_firing = game
        .weapons
        .states
        .get(&actor)
        .map(|state| state.phase == Q2WeaponPhase::Firing)
        .unwrap_or(false);
    let mut context = Q2PlayerContext {
        actor: actor.clone(),
        game,
    };
    let character_weapon = context.weapon_state();
    let game = &mut *context.game;
    let axes = angle_vectors(movement.view_angles);
    let speed = (f64::from(body.velocity.x).powi(2) + f64::from(body.velocity.y).powi(2)).sqrt();
    let snapshot = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .clone();
    let duck = movement.ducked && movement.grounded;
    let bob_time = snapshot.bob_time * if duck { 4.0 } else { 1.0 };
    let cycle = bob_time.trunc() as i64;
    let bob = (bob_time * std::f64::consts::PI).sin().abs();
    let sign = if cycle & 1 != 0 { -1.0 } else { 1.0 };
    let fall = kick_ratio(snapshot.fall_time, now, 0.3, 0.1 - frame);
    let (mut kick_x, mut kick_y, mut kick_z) = (0.0, 0.0, 0.0);
    let mut offset = vec3(0.0, 0.0, 0.0);
    let extra = game
        .rerelease
        .states
        .get(&actor)
        .expect("Q2 rerelease state is missing")
        .clone();
    if !snapshot.dead && !extra.bob_skip {
        let recoil = character_weapon
            .as_ref()
            .map(|weapon| weapon.kick_angles)
            .unwrap_or(vec3(0.0, 0.0, 0.0));
        let damage = kick_ratio(snapshot.damage_time, now, 0.5, 0.1 - frame);
        let velocity_dot = |axis: Vec3| f64::from(dot3(body.velocity, axis));
        kick_x = f64::from(recoil.x)
            + damage * snapshot.damage_pitch
            + fall * snapshot.fall_value
            + velocity_dot(axes.forward) * rules.run_pitch
            + (bob * rules.bob_pitch * speed * if duck { 6.0 } else { 1.0 }).min(1.2);
        kick_y = f64::from(recoil.y);
        kick_z = f64::from(recoil.z)
            + damage * snapshot.damage_roll
            + velocity_dot(axes.right) * rules.run_roll
            + (bob * rules.bob_roll * speed * if duck { 6.0 } else { 1.0 }).min(1.2) * sign;
        if extra.quake_time > now {
            let factor = (extra.quake_time / now * 0.25).min(1.0);
            kick_x += (game.random() * 2.0 - 1.0) * factor;
            kick_y += (game.random() * 2.0 - 1.0) * factor;
            kick_z += (game.random() * 2.0 - 1.0) * factor;
        }
    }
    if !extra.bob_skip {
        let recoil = character_weapon
            .as_ref()
            .map(|weapon| weapon.kick_origin)
            .unwrap_or(vec3(0.0, 0.0, 0.0));
        offset = vec3(
            clamp(f64::from(recoil.x), -14.0, 14.0) as f32,
            clamp(f64::from(recoil.y), -14.0, 14.0) as f32,
            clamp(
                -fall * snapshot.fall_value * 0.4 + (bob * speed * rules.bob_up).min(6.0) + f64::from(recoil.z),
                -22.0,
                30.0,
            ) as f32,
        );
    }
    let mut gun = vec3(0.0, 0.0, 0.0);
    let beam_firing = source_firing
        && character_weapon
            .as_ref()
            .and_then(|weapon| weapon.q2_name.as_deref())
            .is_some_and(|name| name == "heatbeam" || name == "grapple");
    if character_weapon.is_some() && !beam_firing {
        let delta = sub3(snapshot.old_view_angles, base.angles);
        let slow = game
            .rerelease
            .states
            .get(&actor)
            .expect("Q2 rerelease state is missing")
            .slow_view_angles;
        let reduce = |previous: f64, change: f64| {
            let mut value = previous + change;
            if value > 180.0 {
                value -= 360.0;
            }
            if value < -180.0 {
                value += 360.0;
            }
            value = clamp(value, -45.0, 45.0);
            let amount = frame * 1000.0 * if change != 0.0 { 0.05 } else { 0.15 };
            (
                value,
                if value > 0.0 {
                    0.0f64.max(value - amount)
                } else {
                    0.0f64.min(value + amount)
                },
            )
        };
        let (kx, vx) = reduce(f64::from(slow.x), f64::from(delta.x));
        let (ky, vy) = reduce(f64::from(slow.y), f64::from(delta.y));
        let (kz, vz) = reduce(f64::from(slow.z), f64::from(delta.z));
        game.rerelease
            .states
            .get_mut(&actor)
            .expect("Q2 rerelease state is missing")
            .slow_view_angles = vec3(vx as f32, vy as f32, vz as f32);
        gun = vec3(
            (speed * bob * 0.005 + kx * 0.1) as f32,
            (speed * bob * 0.01 * sign + ky * 0.1) as f32,
            (-(speed * bob * 0.005 * sign + kz * 0.05)) as f32,
        );
    }
    Q2PlayerView {
        offset,
        kick_angles: vec3(
            clamp(kick_x, -31.0, 31.0) as f32,
            clamp(kick_y, -31.0, 31.0) as f32,
            clamp(kick_z, -31.0, 31.0) as f32,
        ),
        gun_angles: gun,
        ..base
    }
}

/// Rerelease client animation (`q2RereleaseClientAnimation`).
pub fn q2_rerelease_client_animation(actor: ActorId, game: &mut Q2GameServices) {
    let snapshot = game
        .players
        .states
        .get(&actor)
        .expect("Q2 player has not been admitted")
        .clone();
    let movement = (player_hooks(game).movement)(actor.clone());
    client_animation_inner(game, &actor, &actor, &snapshot, &movement);
}

/// Dummy animation with the source player state (`dummyThink`).
pub fn q2_rerelease_dummy_animation(entity: ActorId, source: ActorId, game: &mut Q2GameServices) {
    let snapshot = game
        .players
        .states
        .get(&source)
        .expect("Q2 player has not been admitted")
        .clone();
    let mut movement = (player_hooks(game).movement)(source.clone());
    movement.grounded = game.body_of(entity.clone()).ground.is_some();
    client_animation_inner(game, &entity, &source, &snapshot, &movement);
}

/// Shared animation driver.
fn client_animation_inner(
    game: &mut Q2GameServices,
    entity: &ActorId,
    state_actor: &ActorId,
    snapshot: &crate::q2::base::player::types::Q2PlayerState,
    movement: &crate::q2::base::player::types::Q2PlayerMovement,
) {
    if !movement.animate_q2 || snapshot.gibbed {
        return;
    }
    let body = game.body_of(entity.clone());
    let run = (f64::from(body.velocity.x).powi(2) + f64::from(body.velocity.y).powi(2)).sqrt() != 0.0;
    let duck = movement.ducked;
    let priority = if snapshot.animation_priority == 6 {
        256
    } else {
        snapshot.animation_priority
    };
    let reversed = priority & 256 != 0;
    let changed = duck != snapshot.animation_duck && priority < 5
        || run != snapshot.animation_run && priority == 0
        || !movement.grounded && priority <= 1;
    let now = game.now();
    if !changed {
        let extra_time = game
            .rerelease
            .states
            .get(state_actor)
            .expect("Q2 rerelease state is missing")
            .animation_time;
        if extra_time > now {
            return;
        }
        let frame = game.require_entity(entity).frame;
        if reversed && frame > snapshot.animation_end || !reversed && frame < snapshot.animation_end {
            game.require_entity_mut(entity).frame = frame + if reversed { -1 } else { 1 };
            game.rerelease
                .states
                .get_mut(state_actor)
                .expect("Q2 rerelease state is missing")
                .animation_time = now + 0.1;
            return;
        }
        if priority == 5 {
            return;
        }
        if priority == 2 {
            if !movement.grounded {
                return;
            }
            {
                let state = game
                    .players
                    .states
                    .get_mut(state_actor)
                    .expect("Q2 player has not been admitted");
                state.animation_priority = if duck { 257 } else { 1 };
                state.animation_end = if duck { 69 } else { 71 };
            }
            game.require_entity_mut(entity).frame = if duck { 71 } else { 68 };
            game.rerelease
                .states
                .get_mut(state_actor)
                .expect("Q2 rerelease state is missing")
                .animation_time = now + 0.1;
            return;
        }
    }
    {
        let state = game
            .players
            .states
            .get_mut(state_actor)
            .expect("Q2 player has not been admitted");
        state.animation_priority = 0;
        state.animation_duck = duck;
        state.animation_run = run;
    }
    game.rerelease
        .states
        .get_mut(state_actor)
        .expect("Q2 rerelease state is missing")
        .animation_time = now + 0.1;
    if !movement.grounded
        && !game
            .rerelease
            .states
            .get(state_actor)
            .expect("Q2 rerelease state is missing")
            .grapple_attached
    {
        let frame = game.require_entity(entity).frame;
        if duck && frame != 155 {
            game.require_entity_mut(entity).frame = 154;
        } else if !duck && frame != 67 {
            game.require_entity_mut(entity).frame = 66;
        }
        let state = game
            .players
            .states
            .get_mut(state_actor)
            .expect("Q2 player has not been admitted");
        state.animation_priority = 2;
        state.animation_end = if duck { 155 } else { 67 };
    } else if movement.grounded && run {
        game.require_entity_mut(entity).frame = if duck { 154 } else { 40 };
        game.players
            .states
            .get_mut(state_actor)
            .expect("Q2 player has not been admitted")
            .animation_end = if duck { 159 } else { 45 };
    } else {
        game.require_entity_mut(entity).frame = if duck { 135 } else { 0 };
        game.players
            .states
            .get_mut(state_actor)
            .expect("Q2 player has not been admitted")
            .animation_end = if duck { 153 } else { 39 };
    }
}
