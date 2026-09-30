//! Quake III presentation: player state.
//!
//! Donor provenance: `src/content/q3/presentation/player-state.ts`.

use qa_core::math::{angle_vectors, dot3, length3, sub3, vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Player-state transitions (player-state.ts)
// ---------------------------------------------------------------------------

pub(crate) const PS_LOCAL_SOUND: i32 = 6;

pub(crate) const PS_ANNOUNCER: i32 = 7;

/// Player-state sounds (`PlayerStateSound` record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct PlayerStateSounds {
    /// No-ammo sound.
    pub no_ammo_sound: Option<PcmSound>,
    /// Hit sound.
    pub hit_sound: Option<PcmSound>,
    /// Team-hit sound.
    pub hit_team_sound: Option<PcmSound>,
    /// Capture award sound.
    pub capture_award_sound: Option<PcmSound>,
    /// Impressive sound.
    pub impressive_sound: Option<PcmSound>,
    /// Excellent sound.
    pub excellent_sound: Option<PcmSound>,
    /// Humiliation sound.
    pub humiliation_sound: Option<PcmSound>,
    /// Defend sound.
    pub defend_sound: Option<PcmSound>,
    /// Assist sound.
    pub assist_sound: Option<PcmSound>,
    /// Denied sound.
    pub denied_sound: Option<PcmSound>,
    /// Holy-shit sound.
    pub holy_shit_sound: Option<PcmSound>,
    /// You-have-flag sound.
    pub you_have_flag_sound: Option<PcmSound>,
    /// Taken-lead sound.
    pub taken_lead_sound: Option<PcmSound>,
    /// Tied-lead sound.
    pub tied_lead_sound: Option<PcmSound>,
    /// Lost-lead sound.
    pub lost_lead_sound: Option<PcmSound>,
    /// Sudden-death sound.
    pub sudden_death_sound: Option<PcmSound>,
    /// One-minute sound.
    pub one_minute_sound: Option<PcmSound>,
    /// Five-minute sound.
    pub five_minute_sound: Option<PcmSound>,
    /// One-frag sound.
    pub one_frag_sound: Option<PcmSound>,
    /// Two-frag sound.
    pub two_frag_sound: Option<PcmSound>,
    /// Three-frag sound.
    pub three_frag_sound: Option<PcmSound>,
}

/// Mission-pack player-state sounds (`MissionPlayerStateSound` record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MissionPlayerStateSounds {
    /// High-armor hit sound.
    pub hit_sound_high_armor: Option<PcmSound>,
    /// Low-armor hit sound.
    pub hit_sound_low_armor: Option<PcmSound>,
    /// First impressive sound.
    pub first_impressive_sound: Option<PcmSound>,
    /// First excellent sound.
    pub first_excellent_sound: Option<PcmSound>,
    /// First humiliation sound.
    pub first_humiliation_sound: Option<PcmSound>,
}

/// Reward medals (`RewardMedal` record).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct RewardMedals {
    /// Capture medal.
    pub medal_capture: Option<SceneShader>,
    /// Impressive medal.
    pub medal_impressive: Option<SceneShader>,
    /// Excellent medal.
    pub medal_excellent: Option<SceneShader>,
    /// Gauntlet medal.
    pub medal_gauntlet: Option<SceneShader>,
    /// Defend medal.
    pub medal_defend: Option<SceneShader>,
    /// Assist medal.
    pub medal_assist: Option<SceneShader>,
}

/// Player-state transition services (`PlayerStateHost`).
pub trait PlayerStateHost {
    /// Product.
    fn product(&self) -> Q3Product;
    /// Show-miss flag.
    fn show_miss(&self) -> bool;
    /// Weapon HUD reader, if the recipe supplies one.
    fn weapon_hud(&mut self) -> Option<(Option<WeaponHudStatus>, ArsenalAmmoWarning)>;
    /// Sounds.
    fn sounds(&self) -> &PlayerStateSounds;
    /// Medals.
    fn medals(&self) -> &RewardMedals;
    /// Mission-pack sounds.
    fn mission_sounds(&self) -> Option<&MissionPlayerStateSounds>;
    /// Dispatch an entity event.
    fn entity_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        position: Vec3,
    ) -> PresentResult<()>;
    /// Play a pain event.
    fn pain_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        health: i32,
    ) -> PresentResult<()>;
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Buffer a sound.
    fn add_buffered_sound(&mut self, sound: Option<PcmSound>);
    /// Print.
    fn print(&mut self, message: &str);
}

/// Player-state transitions (`PlayerStateRuntime`).
pub struct PlayerStateRuntime<H> {
    /// Host services.
    pub host: H,
}

impl<H: PlayerStateHost> PlayerStateRuntime<H> {
    /// New runtime.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self { host }
    }

    fn check_product(&self, state: &ClientGameState, static_state: &ClientGameStaticState) -> PresentResult<()> {
        if state.product != self.host.product() || state.product != static_state.product {
            return Err(state_msg("Player-state transition product mismatch"));
        }
        Ok(())
    }

    fn snapshot_server_time(&self, state: &ClientGameState) -> PresentResult<i32> {
        state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("Player-state transition requires cg.snap"))
            .map(|snapshot| snapshot.server_time)
    }

    fn local(&mut self, sound: Option<PcmSound>, channel: i32) {
        self.host.start_local_sound(sound, channel);
    }

    fn buffered(&mut self, sound: Option<PcmSound>) {
        self.host.add_buffered_sound(sound);
    }

    /// Ammunition warning check (`checkAmmo`).
    pub fn check_ammo(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        if state.product != self.host.product() {
            return Err(state_msg("Player-state transition product mismatch"));
        }
        if let Some((_, warning)) = self.host.weapon_hud() {
            let previous = state.low_ammo_warning;
            state.low_ammo_warning = match warning {
                ArsenalAmmoWarning::Empty => 2,
                ArsenalAmmoWarning::Low => 1,
                ArsenalAmmoWarning::None => 0,
            };
            if state.low_ammo_warning != 0 && state.low_ammo_warning != previous {
                let sound = self.host.sounds().no_ammo_sound;
                self.local(sound, PS_LOCAL_SOUND);
            }
            return Ok(());
        }
        let ps = state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("Player-state transition requires cg.snap"))?
            .player_state
            .clone();
        let weapons = ps.stats.get(stat_schema(ps.product).weapons)?;
        let mut total = 0i32;
        let mut weapon = Weapon::Machinegun as i32;
        while weapon < weapon_count(ps.product) {
            if (weapons & (1 << weapon)) != 0 {
                let slow = weapon == Weapon::RocketLauncher as i32
                    || weapon == Weapon::GrenadeLauncher as i32
                    || weapon == Weapon::Railgun as i32
                    || weapon == Weapon::Shotgun as i32
                    || (ps.product == Q3Product::MissionPack && weapon == Weapon::ProxLauncher as i32);
                total = total.wrapping_add(ps.ammo.get(weapon)?.wrapping_mul(if slow { 1000 } else { 200 }));
                if total >= 5000 {
                    state.low_ammo_warning = 0;
                    return Ok(());
                }
            }
            weapon += 1;
        }
        let previous = state.low_ammo_warning;
        state.low_ammo_warning = if total == 0 { 2 } else { 1 };
        if state.low_ammo_warning != previous {
            let sound = self.host.sounds().no_ammo_sound;
            self.local(sound, PS_LOCAL_SOUND);
        }
        Ok(())
    }

    /// Damage feedback (`damageFeedback`).
    pub fn damage_feedback(
        &mut self,
        state: &mut ClientGameState,
        yaw_byte: i32,
        pitch_byte: i32,
        damage: i32,
    ) -> PresentResult<()> {
        let snapshot = state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("Player-state transition requires cg.snap"))?;
        let health = snapshot.player_state.health()?;
        let server_time = snapshot.server_time;
        state.attacker_time = state.time;
        let scale = if health < 40 { 1.0 } else { 40.0 / health as f32 };
        let mut kick = damage as f32 * scale;
        kick = kick.clamp(5.0, 10.0);
        if yaw_byte == 255 && pitch_byte == 255 {
            state.damage_x = 0.0;
            state.damage_y = 0.0;
            state.damage_roll = 0.0;
            state.damage_pitch = -kick;
        } else {
            let pitch = pitch_byte as f32 / 255.0 * 360.0;
            let yaw = yaw_byte as f32 / 255.0 * 360.0;
            let direction = sub3(vec3(0.0, 0.0, 0.0), angle_vectors(vec3(pitch, yaw, 0.0)).forward);
            let mut front = dot3(direction, state.refdef.view_axis[0]);
            let left = dot3(direction, state.refdef.view_axis[1]);
            let up = dot3(direction, state.refdef.view_axis[2]);
            let mut distance = length3(vec3(front, left, 0.0));
            if distance < 0.1 {
                distance = 0.1;
            }
            state.damage_roll = kick * left;
            state.damage_pitch = -kick * front;
            if front <= 0.1 {
                front = 0.1;
            }
            state.damage_x = -left / front;
            state.damage_y = up / distance;
        }
        state.damage_x = state.damage_x.clamp(-1.0, 1.0);
        state.damage_y = state.damage_y.clamp(-1.0, 1.0);
        if kick > 10.0 {
            kick = 10.0;
        }
        state.damage_value = kick;
        state.damage_kick_end_time = state.time.wrapping_add(500) as f32;
        state.damage_time = server_time as f32;
        Ok(())
    }

    /// Respawn presentation (`respawn`).
    pub fn respawn(&mut self, state: &mut ClientGameState) -> PresentResult<()> {
        state.this_frame_teleport = true;
        state.weapon_select_time = state.time;
        let weapon = state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("Player-state transition requires cg.snap"))?
            .player_state
            .weapon;
        state.weapon_select = weapon;
        Ok(())
    }

    /// Player-state event check (`checkPlayerstateEvents`).
    pub fn check_playerstate_events(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        current: &PlayerState,
        previous: &PlayerState,
    ) -> PresentResult<()> {
        if current.external_event != 0 && current.external_event != previous.external_event {
            let number = usize::try_from(current.client_num).map_err(|_| range_msg("Entity number outside int32"))?;
            let entity = event_entity_mut(state, EventEntityRef::Entity(number))?;
            entity.current_state.event = current.external_event;
            entity.current_state.event_parm = current.external_event_parm;
            let position = entity.lerp_origin;
            self.host
                .entity_event(state, static_state, EventEntityRef::Entity(number), position)?;
        }
        let mut index = current.event_sequence.wrapping_sub(2);
        while index < current.event_sequence {
            if index >= previous.event_sequence
                || (index > previous.event_sequence.wrapping_sub(2)
                    && current.events.get(index & 1)? != previous.events.get(index & 1)?)
            {
                let event = current.events.get(index & 1)?;
                let parm = current.event_parms.get(index & 1)?;
                let entity = event_entity_mut(state, EventEntityRef::PredictedPlayer)?;
                entity.current_state.event = event;
                entity.current_state.event_parm = parm;
                let position = entity.lerp_origin;
                self.host
                    .entity_event(state, static_state, EventEntityRef::PredictedPlayer, position)?;
                state.predictable_events.set(index & 15, event)?;
                state.event_sequence = state.event_sequence.wrapping_add(1);
            }
            index = index.wrapping_add(1);
        }
        Ok(())
    }

    /// Changed predictable-event check (`checkChangedPredictableEvents`).
    pub fn check_changed_predictable_events(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        ps: &PlayerState,
    ) -> PresentResult<()> {
        let mut index = ps.event_sequence.wrapping_sub(2);
        while index < ps.event_sequence {
            if index < state.event_sequence
                && index > state.event_sequence.wrapping_sub(16)
                && ps.events.get(index & 1)? != state.predictable_events.get(index & 15)?
            {
                let event = ps.events.get(index & 1)?;
                let parm = ps.event_parms.get(index & 1)?;
                let entity = event_entity_mut(state, EventEntityRef::PredictedPlayer)?;
                entity.current_state.event = event;
                entity.current_state.event_parm = parm;
                let position = entity.lerp_origin;
                self.host
                    .entity_event(state, static_state, EventEntityRef::PredictedPlayer, position)?;
                state.predictable_events.set(index & 15, event)?;
                if self.host.show_miss() {
                    self.host.print("WARNING: changed predicted event\n");
                }
            }
            index = index.wrapping_add(1);
        }
        Ok(())
    }

    fn push_reward(
        state: &mut ClientGameState,
        sound: Option<PcmSound>,
        shader: Option<SceneShader>,
        count: i32,
    ) -> PresentResult<()> {
        if state.reward_stack < 9 {
            state.reward_stack = state.reward_stack.wrapping_add(1);
            let reward = at_mut(
                &mut state.rewards,
                state.reward_stack as usize,
                "Invalid source reward stack index",
            )?;
            reward.sound = sound;
            reward.shader = shader;
            reward.count = count;
        }
        Ok(())
    }

    /// Local sounds (`checkLocalSounds`).
    pub fn check_local_sounds(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        ps: &PlayerState,
        previous: &PlayerState,
    ) -> PresentResult<()> {
        self.check_product(state, static_state)?;
        if ps.persistant.get(PersistentIndex::Team as i32)? != previous.persistant.get(PersistentIndex::Team as i32)? {
            return Ok(());
        }
        let mission = self.host.product() == Q3Product::MissionPack;
        if ps.persistant.get(PersistentIndex::Hits as i32)? > previous.persistant.get(PersistentIndex::Hits as i32)? {
            let packed = ps.persistant.get(PersistentIndex::AttackeeArmor as i32)?;
            let armor = packed & 255;
            let health = packed >> 8;
            if mission && armor > 50 {
                let sound = self
                    .host
                    .mission_sounds()
                    .and_then(|sounds| sounds.hit_sound_high_armor);
                self.local(sound, PS_LOCAL_SOUND);
            } else if mission && (armor != 0 || health > 100) {
                let sound = self.host.mission_sounds().and_then(|sounds| sounds.hit_sound_low_armor);
                self.local(sound, PS_LOCAL_SOUND);
            } else {
                let sound = self.host.sounds().hit_sound;
                self.local(sound, PS_LOCAL_SOUND);
            }
        } else if ps.persistant.get(PersistentIndex::Hits as i32)?
            < previous.persistant.get(PersistentIndex::Hits as i32)?
        {
            let sound = self.host.sounds().hit_team_sound;
            self.local(sound, PS_LOCAL_SOUND);
        }
        if ps.health()? < previous.health()?.wrapping_sub(1) && ps.health()? > 0 {
            self.host
                .pain_event(state, static_state, EventEntityRef::PredictedPlayer, ps.health()?)?;
        }
        if state.intermission_started {
            return Ok(());
        }
        let mut rewarded = false;
        let changed = |index: PersistentIndex| -> PresentResult<bool> {
            Ok(ps.persistant.get(index as i32)? != previous.persistant.get(index as i32)?)
        };
        if changed(PersistentIndex::Captures)? {
            let sounds = *self.host.sounds();
            let medals = self.host.medals().clone();
            Self::push_reward(
                state,
                sounds.capture_award_sound,
                medals.medal_capture,
                ps.persistant.get(PersistentIndex::Captures as i32)?,
            )?;
            rewarded = true;
        }
        if changed(PersistentIndex::ImpressiveCount)? {
            let count = ps.persistant.get(PersistentIndex::ImpressiveCount as i32)?;
            let sounds = *self.host.sounds();
            let medals = self.host.medals().clone();
            let sound = if mission && count == 1 {
                self.host
                    .mission_sounds()
                    .and_then(|sounds| sounds.first_impressive_sound)
            } else {
                sounds.impressive_sound
            };
            Self::push_reward(state, sound, medals.medal_impressive, count)?;
            rewarded = true;
        }
        if changed(PersistentIndex::ExcellentCount)? {
            let count = ps.persistant.get(PersistentIndex::ExcellentCount as i32)?;
            let sounds = *self.host.sounds();
            let medals = self.host.medals().clone();
            let sound = if mission && count == 1 {
                self.host
                    .mission_sounds()
                    .and_then(|sounds| sounds.first_excellent_sound)
            } else {
                sounds.excellent_sound
            };
            Self::push_reward(state, sound, medals.medal_excellent, count)?;
            rewarded = true;
        }
        if changed(PersistentIndex::GauntletFragCount)? {
            let count = ps.persistant.get(PersistentIndex::GauntletFragCount as i32)?;
            let sounds = *self.host.sounds();
            let medals = self.host.medals().clone();
            let sound = if mission && previous.persistant.get(PersistentIndex::GauntletFragCount as i32)? == 1 {
                self.host
                    .mission_sounds()
                    .and_then(|sounds| sounds.first_humiliation_sound)
            } else {
                sounds.humiliation_sound
            };
            Self::push_reward(state, sound, medals.medal_gauntlet, count)?;
            rewarded = true;
        }
        if changed(PersistentIndex::DefendCount)? {
            let sounds = *self.host.sounds();
            let medals = self.host.medals().clone();
            Self::push_reward(
                state,
                sounds.defend_sound,
                medals.medal_defend,
                ps.persistant.get(PersistentIndex::DefendCount as i32)?,
            )?;
            rewarded = true;
        }
        if changed(PersistentIndex::AssistCount)? {
            let sounds = *self.host.sounds();
            let medals = self.host.medals().clone();
            Self::push_reward(
                state,
                sounds.assist_sound,
                medals.medal_assist,
                ps.persistant.get(PersistentIndex::AssistCount as i32)?,
            )?;
            rewarded = true;
        }
        if changed(PersistentIndex::PlayerEvents)? {
            let changed_bits = ps.persistant.get(PersistentIndex::PlayerEvents as i32)?
                ^ previous.persistant.get(PersistentIndex::PlayerEvents as i32)?;
            if (changed_bits & 1) != 0 {
                let sound = self.host.sounds().denied_sound;
                self.local(sound, PS_ANNOUNCER);
            } else if (changed_bits & 2) != 0 {
                let sound = self.host.sounds().humiliation_sound;
                self.local(sound, PS_ANNOUNCER);
            } else if (changed_bits & 4) != 0 {
                let sound = self.host.sounds().holy_shit_sound;
                self.local(sound, PS_ANNOUNCER);
            }
            rewarded = true;
        }
        if static_state.game_type >= GameType::Team {
            for powerup in [Powerup::RedFlag, Powerup::BlueFlag, Powerup::NeutralFlag] {
                if ps.powerups.get(powerup as i32)? != previous.powerups.get(powerup as i32)?
                    && ps.powerups.get(powerup as i32)? != 0
                {
                    let sound = self.host.sounds().you_have_flag_sound;
                    self.local(sound, PS_ANNOUNCER);
                    break;
                }
            }
        }
        if !rewarded && state.warmup == 0 && changed(PersistentIndex::Rank)? && static_state.game_type < GameType::Team
        {
            let rank = ps.persistant.get(PersistentIndex::Rank as i32)?;
            if rank == 0 {
                let sound = self.host.sounds().taken_lead_sound;
                self.buffered(sound);
            } else if rank == 0x4000 {
                let sound = self.host.sounds().tied_lead_sound;
                self.buffered(sound);
            } else if (previous.persistant.get(PersistentIndex::Rank as i32)? & !0x4000) == 0 {
                let sound = self.host.sounds().lost_lead_sound;
                self.buffered(sound);
            }
        }
        if static_state.timelimit > 0 {
            let msec = state.time.wrapping_sub(static_state.level_start_time);
            if (state.timelimit_warnings & 4) == 0
                && msec
                    > static_state
                        .timelimit
                        .wrapping_mul(60)
                        .wrapping_add(2)
                        .wrapping_mul(1000)
            {
                state.timelimit_warnings |= 7;
                let sound = self.host.sounds().sudden_death_sound;
                self.local(sound, PS_ANNOUNCER);
            } else if (state.timelimit_warnings & 2) == 0
                && msec
                    > static_state
                        .timelimit
                        .wrapping_sub(1)
                        .wrapping_mul(60)
                        .wrapping_mul(1000)
            {
                state.timelimit_warnings |= 3;
                let sound = self.host.sounds().one_minute_sound;
                self.local(sound, PS_ANNOUNCER);
            } else if static_state.timelimit > 5
                && (state.timelimit_warnings & 1) == 0
                && msec
                    > static_state
                        .timelimit
                        .wrapping_sub(5)
                        .wrapping_mul(60)
                        .wrapping_mul(1000)
            {
                state.timelimit_warnings |= 1;
                let sound = self.host.sounds().five_minute_sound;
                self.local(sound, PS_ANNOUNCER);
            }
        }
        if static_state.fraglimit > 0 && static_state.game_type < GameType::Ctf {
            if (state.fraglimit_warnings & 4) == 0 && static_state.scores1 == static_state.fraglimit.wrapping_sub(1) {
                state.fraglimit_warnings |= 7;
                let sound = self.host.sounds().one_frag_sound;
                self.buffered(sound);
            } else if static_state.fraglimit > 2
                && (state.fraglimit_warnings & 2) == 0
                && static_state.scores1 == static_state.fraglimit.wrapping_sub(2)
            {
                state.fraglimit_warnings |= 3;
                let sound = self.host.sounds().two_frag_sound;
                self.buffered(sound);
            } else if static_state.fraglimit > 3
                && (state.fraglimit_warnings & 1) == 0
                && static_state.scores1 == static_state.fraglimit.wrapping_sub(3)
            {
                state.fraglimit_warnings |= 1;
                let sound = self.host.sounds().three_frag_sound;
                self.buffered(sound);
            }
        }
        Ok(())
    }

    /// Player-state transition (`transitionPlayerState`).
    pub fn transition_player_state(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        current: &PlayerState,
        previous: &mut PlayerState,
    ) -> PresentResult<()> {
        if current.client_num != previous.client_num {
            state.this_frame_teleport = true;
            previous.copy_from_state(current)?;
        }
        if current.damage_event != previous.damage_event && current.damage_count != 0 {
            self.damage_feedback(state, current.damage_yaw, current.damage_pitch, current.damage_count)?;
        }
        if current.persistant.get(PersistentIndex::SpawnCount as i32)?
            != previous.persistant.get(PersistentIndex::SpawnCount as i32)?
        {
            self.respawn(state)?;
        }
        if state.map_restart {
            self.respawn(state)?;
            state.map_restart = false;
        }
        let snapshot_pm = state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("Player-state transition requires cg.snap"))?
            .player_state
            .pm_type;
        if snapshot_pm != MoveType::Intermission as i32
            && current.persistant.get(PersistentIndex::Team as i32)? != Team::Spectator as i32
        {
            self.check_local_sounds(state, static_state, current, previous)?;
        }
        self.check_ammo(state)?;
        self.check_playerstate_events(state, static_state, current, previous)?;
        if current.viewheight != previous.viewheight {
            state.duck_change = current.viewheight.wrapping_sub(previous.viewheight) as f32;
            state.duck_time = state.time;
        }
        let _ = self.snapshot_server_time(state)?;
        Ok(())
    }
}
