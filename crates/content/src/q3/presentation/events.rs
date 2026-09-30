//! Quake III presentation: events.
//!
//! Donor provenance: `src/content/q3/presentation/events.ts`.

use crate::q3anim::{PlayerFootsteps, PlayerGender};
use qa_core::math::{vec3, vec4, Vec3, Vec4};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::presentation::mirrors_present_client::*;
use crate::q3::presentation::retail_snapshot::*;
use crate::q3::presentation::state::*;

// ---------------------------------------------------------------------------
// Entity events (events.ts)
// ---------------------------------------------------------------------------

pub(crate) const EV_AUTO: i32 = 0;

pub(crate) const EV_VOICE: i32 = 3;

pub(crate) const EV_ITEM: i32 = 4;

pub(crate) const EV_BODY: i32 = 5;

pub(crate) const EV_ANNOUNCER: i32 = 7;

/// Player-state event sounds (`ClientEventSound` record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClientEventSounds {
    /// Use-nothing sound.
    pub use_nothing_sound: Option<PcmSound>,
    /// Medkit sound.
    pub medkit_sound: Option<PcmSound>,
    /// Land sound.
    pub land_sound: Option<PcmSound>,
    /// Jump-pad sound.
    pub jump_pad_sound: Option<PcmSound>,
    /// Water-in sound.
    pub watr_in_sound: Option<PcmSound>,
    /// Water-out sound.
    pub watr_out_sound: Option<PcmSound>,
    /// Water-under sound.
    pub watr_un_sound: Option<PcmSound>,
    /// Health sound.
    pub n_health_sound: Option<PcmSound>,
    /// Select sound.
    pub select_sound: Option<PcmSound>,
    /// Teleport-in sound.
    pub tele_in_sound: Option<PcmSound>,
    /// Teleport-out sound.
    pub tele_out_sound: Option<PcmSound>,
    /// Respawn sound.
    pub respawn_sound: Option<PcmSound>,
    /// Grenade bounce 1.
    pub hgrenb1a_sound: Option<PcmSound>,
    /// Grenade bounce 2.
    pub hgrenb2a_sound: Option<PcmSound>,
    /// Capture-your-team sound.
    pub capture_your_team_sound: Option<PcmSound>,
    /// Capture-opponent sound.
    pub capture_opponent_sound: Option<PcmSound>,
    /// Return-your-team sound.
    pub return_your_team_sound: Option<PcmSound>,
    /// Return-opponent sound.
    pub return_opponent_sound: Option<PcmSound>,
    /// Blue-flag-returned sound.
    pub blue_flag_returned_sound: Option<PcmSound>,
    /// Red-flag-returned sound.
    pub red_flag_returned_sound: Option<PcmSound>,
    /// Enemy-took-your-flag sound.
    pub enemy_took_your_flag_sound: Option<PcmSound>,
    /// Your-team-took-enemy-flag sound.
    pub your_team_took_enemy_flag_sound: Option<PcmSound>,
    /// Your-base-under-attack sound.
    pub your_base_is_under_attack_sound: Option<PcmSound>,
    /// Red-scored sound.
    pub red_scored_sound: Option<PcmSound>,
    /// Blue-scored sound.
    pub blue_scored_sound: Option<PcmSound>,
    /// Red-leads sound.
    pub red_leads_sound: Option<PcmSound>,
    /// Blue-leads sound.
    pub blue_leads_sound: Option<PcmSound>,
    /// Teams-tied sound.
    pub teams_tied_sound: Option<PcmSound>,
    /// Quad sound.
    pub quad_sound: Option<PcmSound>,
    /// Protect sound.
    pub protect_sound: Option<PcmSound>,
    /// Regen sound.
    pub regen_sound: Option<PcmSound>,
    /// Gib sound.
    pub gib_sound: Option<PcmSound>,
}

/// Mission-pack event sounds (`MissionEventSound` record).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MissionEventSounds {
    /// Use-invulnerability sound.
    pub use_invulnerability_sound: Option<PcmSound>,
    /// Scout sound.
    pub scout_sound: Option<PcmSound>,
    /// Guard sound.
    pub guard_sound: Option<PcmSound>,
    /// Doubler sound.
    pub doubler_sound: Option<PcmSound>,
    /// Ammo-regen sound.
    pub ammoregen_sound: Option<PcmSound>,
    /// Prox-mine stick (flesh).
    pub wstbimpl_sound: Option<PcmSound>,
    /// Prox-mine stick (metal).
    pub wstbimpm_sound: Option<PcmSound>,
    /// Prox-mine stick (default).
    pub wstbimpd_sound: Option<PcmSound>,
    /// Prox-mine trigger.
    pub wstbactv_sound: Option<PcmSound>,
    /// Your-team-took-the-flag sound.
    pub your_team_took_the_flag_sound: Option<PcmSound>,
    /// Enemy-took-the-flag sound.
    pub enemy_took_the_flag_sound: Option<PcmSound>,
    /// Kamikaze-far sound.
    pub kamikaze_far_sound: Option<PcmSound>,
}

/// Footstep sound bank keyed by surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FootstepBank {
    /// Normal.
    pub normal: [Option<PcmSound>; 4],
    /// Boot.
    pub boot: [Option<PcmSound>; 4],
    /// Flesh.
    pub flesh: [Option<PcmSound>; 4],
    /// Mech.
    pub mech: [Option<PcmSound>; 4],
    /// Energy.
    pub energy: [Option<PcmSound>; 4],
    /// Metal.
    pub metal: [Option<PcmSound>; 4],
    /// Splash.
    pub splash: [Option<PcmSound>; 4],
}

impl FootstepBank {
    /// Bank for a footstep style.
    #[must_use]
    pub const fn bank(&self, footsteps: PlayerFootsteps) -> &[Option<PcmSound>; 4] {
        match footsteps {
            PlayerFootsteps::Normal => &self.normal,
            PlayerFootsteps::Boot => &self.boot,
            PlayerFootsteps::Flesh => &self.flesh,
            PlayerFootsteps::Mech => &self.mech,
            PlayerFootsteps::Energy => &self.energy,
        }
    }
}

/// Client event media (`ClientEventMedia`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEventMedia {
    /// Sounds.
    pub sounds: ClientEventSounds,
    /// Footsteps.
    pub footsteps: FootstepBank,
    /// Game sounds.
    pub game_sounds: Vec<Option<PcmSound>>,
    /// Smoke-puff shader.
    pub smoke_puff_shader: Option<SceneShader>,
}

/// Client event options (`ClientEventOptions`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientEventOptions {
    /// Game type.
    pub game_type: GameType,
    /// Debug events.
    pub debug_events: bool,
    /// Footsteps enabled.
    pub footsteps: bool,
    /// Autoswitch.
    pub autoswitch: bool,
    /// Demo playback.
    pub demo_playback: bool,
    /// No predict.
    pub no_predict: bool,
    /// Synchronous clients.
    pub synchronous_clients: bool,
    /// Single-player active.
    pub single_player_active: bool,
    /// Camera orbit.
    pub camera_orbit: bool,
}

/// Brief client info for events.
#[derive(Debug, Clone, Copy)]
pub struct EventClientInfo {
    /// Gender.
    pub gender: PlayerGender,
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Team.
    pub team: Team,
}

/// Missile impact sound (`ImpactSound`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImpactSound {
    /// Default.
    Default,
    /// Metal.
    Metal,
}

/// Bullet target (`weapons.bullet` target).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BulletTarget {
    /// Wall hit with normal.
    Wall {
        /// Normal.
        normal: Vec3,
    },
    /// Flesh hit with entity number.
    Flesh {
        /// Entity number.
        entity_num: i32,
    },
}

/// Smoke-puff descriptor.
#[derive(Debug, Clone, PartialEq)]
pub struct SmokePuffDesc {
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

/// Spawned smoke puff handle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnedPuff {
    /// Local-entity type override.
    pub le_type: Option<String>,
}

/// Entity event services (`ClientEventHost`).
#[allow(clippy::too_many_arguments)]
pub trait ClientEventHost {
    /// Product.
    fn product(&self) -> Q3Product;
    /// Media.
    fn media(&self) -> &ClientEventMedia;
    /// Options.
    fn options(&self) -> ClientEventOptions;
    /// Mission sounds.
    fn mission_sounds(&self) -> Option<&MissionEventSounds>;
    /// Random integer (`rand`).
    fn rand_int(&mut self) -> i32;
    /// Recipe pre-hook; `false` skips the source event.
    fn pre_present_event(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef, position: Vec3) -> bool;
    /// Recipe post-hook.
    fn post_present_event(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef, position: Vec3);
    /// Set the entity sound position.
    fn set_entity_sound_position(&mut self, state: &ClientGameState, entity_number: usize);
    /// Debug beam.
    fn beam(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef);
    /// Out-of-ammo weapon change.
    fn out_of_ammo_change(&mut self, state: &mut ClientGameState);
    /// Fire weapon effect.
    fn fire_weapon(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef);
    /// Missile-hit-player effect.
    fn missile_hit_player(
        &mut self,
        state: &mut ClientGameState,
        weapon: i32,
        position: Vec3,
        direction: Vec3,
        other_entity_num: i32,
    );
    /// Missile-hit-wall effect.
    fn missile_hit_wall(
        &mut self,
        state: &mut ClientGameState,
        weapon: i32,
        client_num: i32,
        position: Vec3,
        direction: Vec3,
        impact: ImpactSound,
    );
    /// Rail trail effect.
    fn rail_trail(&mut self, state: &mut ClientGameState, client_num: i32, origin2: Vec3, base: Vec3);
    /// Bullet effect.
    fn bullet(&mut self, state: &mut ClientGameState, base: Vec3, other_entity_num: i32, target: BulletTarget);
    /// Shotgun effect.
    fn shotgun_fire(&mut self, state: &mut ClientGameState, entity_ref: EventEntityRef);
    /// Smoke puff.
    fn smoke_puff(
        &mut self,
        state: &mut ClientGameState,
        desc: &SmokePuffDesc,
        le_type_override: Option<&str>,
    ) -> SpawnedPuff;
    /// Spawn effect.
    fn spawn_effect(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Gib effect.
    fn gib_player(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Score plum.
    fn score_plum(&mut self, state: &mut ClientGameState, other_entity_num: i32, position: Vec3, time: i32);
    /// Kamikaze effect.
    fn kamikaze_effect(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Obelisk explosion.
    fn obelisk_explode(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Obelisk pain.
    fn obelisk_pain(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Invulnerability impact.
    fn invulnerability_impact(&mut self, state: &mut ClientGameState, position: Vec3, angles: Vec3);
    /// Invulnerability juiced.
    fn invulnerability_juiced(&mut self, state: &mut ClientGameState, position: Vec3);
    /// Lightning-bolt beam.
    fn lightning_bolt_beam(&mut self, state: &mut ClientGameState, origin2: Vec3, base: Vec3);
    /// Brief client info.
    fn client_info_brief(&self, static_state: &ClientGameStaticState, number: i32) -> PresentResult<EventClientInfo>;
    /// Record medkit usage time.
    fn set_medkit_usage_time(
        &mut self,
        static_state: &mut ClientGameStaticState,
        client_num: i32,
        time: i32,
    ) -> PresentResult<()>;
    /// Live player name, if the configstring exists.
    fn player_name(&self, number: i32) -> Option<String>;
    /// Sound configstring.
    fn sound_config_string(&self, index: i32) -> String;
    /// Custom sound.
    fn custom_sound(&mut self, client_num: i32, name: &str) -> Option<PcmSound>;
    /// Register a sound synchronously.
    fn register_sound(&mut self, path: Option<&str>, compressed: bool) -> Option<PcmSound>;
    /// Start a sound.
    fn start_sound(&mut self, origin: Option<Vec3>, entity_num: i32, channel: i32, sound: Option<PcmSound>);
    /// Start a local sound.
    fn start_local_sound(&mut self, sound: Option<PcmSound>, channel: i32);
    /// Stop a looping sound.
    fn stop_looping_sound(&mut self, entity_num: i32);
    /// Buffer a sound.
    fn add_buffered_sound(&mut self, sound: Option<PcmSound>);
    /// Print.
    fn print(&mut self, message: &str);
    /// Center print.
    fn center_print(&mut self, message: &str, y: i32, char_width: i32);
    /// Local voice chat.
    fn voice_chat_local(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        mode: i32,
        voice_only: bool,
        client_num: i32,
        color: i32,
        command: &str,
    ) -> PresentResult<()>;
}

/// Rank place string (`placeString`).
pub fn place_string(rank: i32) -> PresentResult<String> {
    let tied = (rank & 0x4000) != 0;
    let rank = rank & !0x4000;
    let place = if rank == 1 {
        "^41st^7".to_owned()
    } else if rank == 2 {
        "^12nd^7".to_owned()
    } else if rank == 3 {
        "^33rd^7".to_owned()
    } else {
        let suffix = if rank == 11 || rank == 12 || rank == 13 {
            "th"
        } else if rank % 10 == 1 {
            "st"
        } else if rank % 10 == 2 {
            "nd"
        } else if rank % 10 == 3 {
            "rd"
        } else {
            "th"
        };
        game_format(&format!("%i{suffix}"), &[GameFormatArg::from(rank)], 16_384)?
    };
    game_format(
        "%s%s",
        &[
            GameFormatArg::from(if tied { "Tied for " } else { "" }),
            GameFormatArg::from(place),
        ],
        64,
    )
}

pub(crate) fn entity_event_name(event: i32) -> Option<&'static str> {
    EntityEvent::from_i32(event).map(|event| match event {
        EntityEvent::None => "EV_NONE",
        EntityEvent::Footstep => "EV_FOOTSTEP",
        EntityEvent::FootstepMetal => "EV_FOOTSTEP_METAL",
        EntityEvent::Footsplash => "EV_FOOTSPLASH",
        EntityEvent::Footwade => "EV_FOOTWADE",
        EntityEvent::Swim => "EV_SWIM",
        EntityEvent::Step4 => "EV_STEP_4",
        EntityEvent::Step8 => "EV_STEP_8",
        EntityEvent::Step12 => "EV_STEP_12",
        EntityEvent::Step16 => "EV_STEP_16",
        EntityEvent::FallShort => "EV_FALL_SHORT",
        EntityEvent::FallMedium => "EV_FALL_MEDIUM",
        EntityEvent::FallFar => "EV_FALL_FAR",
        EntityEvent::JumpPad => "EV_JUMP_PAD",
        EntityEvent::Jump => "EV_JUMP",
        EntityEvent::WaterTouch => "EV_WATER_TOUCH",
        EntityEvent::WaterLeave => "EV_WATER_LEAVE",
        EntityEvent::WaterUnder => "EV_WATER_UNDER",
        EntityEvent::WaterClear => "EV_WATER_CLEAR",
        EntityEvent::ItemPickup => "EV_ITEM_PICKUP",
        EntityEvent::GlobalItemPickup => "EV_GLOBAL_ITEM_PICKUP",
        EntityEvent::Noammo => "EV_NOAMMO",
        EntityEvent::ChangeWeapon => "EV_CHANGE_WEAPON",
        EntityEvent::FireWeapon => "EV_FIRE_WEAPON",
        EntityEvent::UseItem0 => "EV_USE_ITEM0",
        EntityEvent::UseItem1 => "EV_USE_ITEM1",
        EntityEvent::UseItem2 => "EV_USE_ITEM2",
        EntityEvent::UseItem3 => "EV_USE_ITEM3",
        EntityEvent::UseItem4 => "EV_USE_ITEM4",
        EntityEvent::UseItem5 => "EV_USE_ITEM5",
        EntityEvent::UseItem6 => "EV_USE_ITEM6",
        EntityEvent::UseItem7 => "EV_USE_ITEM7",
        EntityEvent::UseItem8 => "EV_USE_ITEM8",
        EntityEvent::UseItem9 => "EV_USE_ITEM9",
        EntityEvent::UseItem10 => "EV_USE_ITEM10",
        EntityEvent::UseItem11 => "EV_USE_ITEM11",
        EntityEvent::UseItem12 => "EV_USE_ITEM12",
        EntityEvent::UseItem13 => "EV_USE_ITEM13",
        EntityEvent::UseItem14 => "EV_USE_ITEM14",
        EntityEvent::UseItem15 => "EV_USE_ITEM15",
        EntityEvent::ItemRespawn => "EV_ITEM_RESPAWN",
        EntityEvent::ItemPop => "EV_ITEM_POP",
        EntityEvent::PlayerTeleportIn => "EV_PLAYER_TELEPORT_IN",
        EntityEvent::PlayerTeleportOut => "EV_PLAYER_TELEPORT_OUT",
        EntityEvent::GrenadeBounce => "EV_GRENADE_BOUNCE",
        EntityEvent::GeneralSound => "EV_GENERAL_SOUND",
        EntityEvent::GlobalSound => "EV_GLOBAL_SOUND",
        EntityEvent::GlobalTeamSound => "EV_GLOBAL_TEAM_SOUND",
        EntityEvent::BulletHitFlesh => "EV_BULLET_HIT_FLESH",
        EntityEvent::BulletHitWall => "EV_BULLET_HIT_WALL",
        EntityEvent::MissileHit => "EV_MISSILE_HIT",
        EntityEvent::MissileMiss => "EV_MISSILE_MISS",
        EntityEvent::MissileMissMetal => "EV_MISSILE_MISS_METAL",
        EntityEvent::Railtrail => "EV_RAILTRAIL",
        EntityEvent::Shotgun => "EV_SHOTGUN",
        EntityEvent::Bullet => "EV_BULLET",
        EntityEvent::Pain => "EV_PAIN",
        EntityEvent::Death1 => "EV_DEATH1",
        EntityEvent::Death2 => "EV_DEATH2",
        EntityEvent::Death3 => "EV_DEATH3",
        EntityEvent::Obituary => "EV_OBITUARY",
        EntityEvent::PowerupQuad => "EV_POWERUP_QUAD",
        EntityEvent::PowerupBattlesuit => "EV_POWERUP_BATTLESUIT",
        EntityEvent::PowerupRegen => "EV_POWERUP_REGEN",
        EntityEvent::GibPlayer => "EV_GIB_PLAYER",
        EntityEvent::Scoreplum => "EV_SCOREPLUM",
        EntityEvent::ProximityMineStick => "EV_PROXIMITY_MINE_STICK",
        EntityEvent::ProximityMineTrigger => "EV_PROXIMITY_MINE_TRIGGER",
        EntityEvent::Kamikaze => "EV_KAMIKAZE",
        EntityEvent::ObeliskExplode => "EV_OBELISKEXPLODE",
        EntityEvent::ObeliskPain => "EV_OBELISKPAIN",
        EntityEvent::InvulImpact => "EV_INVUL_IMPACT",
        EntityEvent::Juiced => "EV_JUICED",
        EntityEvent::LightningBolt => "EV_LIGHTNINGBOLT",
        EntityEvent::DebugLine => "EV_DEBUG_LINE",
        EntityEvent::StopLoopingSound => "EV_STOPLOOPINGSOUND",
        EntityEvent::Taunt => "EV_TAUNT",
        EntityEvent::TauntYes => "EV_TAUNT_YES",
        EntityEvent::TauntNo => "EV_TAUNT_NO",
        EntityEvent::TauntFollowMe => "EV_TAUNT_FOLLOWME",
        EntityEvent::TauntGetFlag => "EV_TAUNT_GETFLAG",
        EntityEvent::TauntGuardBase => "EV_TAUNT_GUARDBASE",
        EntityEvent::TauntPatrol => "EV_TAUNT_PATROL",
    })
}

/// Client entity events (`ClientEventRuntime`).
pub struct ClientEventRuntime<H> {
    /// Host services.
    pub host: H,
}

impl<H: ClientEventHost> ClientEventRuntime<H> {
    /// New runtime.
    #[must_use]
    pub fn new(host: H) -> Self {
        Self { host }
    }

    fn check_product(&self, state: &ClientGameState) -> PresentResult<()> {
        if state.product != self.host.product() {
            return Err(state_msg("Event host product differs from cgame state"));
        }
        Ok(())
    }

    fn snapshot<'a>(&self, state: &'a ClientGameState) -> PresentResult<&'a Snapshot> {
        state
            .snap
            .as_ref()
            .ok_or_else(|| state_msg("CG event requires cg.snap"))
    }

    fn center_y(&self, state: &ClientGameState) -> i32 {
        match state.product {
            Q3Product::BaseQ3 => 143,
            Q3Product::MissionPack => 144,
        }
    }

    /// Pain event (`painEvent`).
    pub fn pain_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        health: i32,
    ) -> PresentResult<()> {
        self.check_product(state)?;
        let _ = static_state;
        let time = state.time;
        let entity = event_entity_mut(state, entity_ref)?;
        if time.wrapping_sub(entity.player.pain_time) < 500 {
            return Ok(());
        }
        let level = if health < 25 {
            25
        } else if health < 50 {
            50
        } else if health < 75 {
            75
        } else {
            100
        };
        let number = entity.current_state.number;
        let sound = self.host.custom_sound(number, &format!("*pain{level}_1.wav"));
        self.host.start_sound(None, number, EV_VOICE, sound);
        let entity = event_entity_mut(state, entity_ref)?;
        entity.player.pain_time = time;
        entity.player.pain_direction = !entity.player.pain_direction;
        Ok(())
    }

    fn item_pickup(&mut self, state: &mut ClientGameState, index: i32) -> PresentResult<()> {
        state.item_pickup = index;
        state.item_pickup_time = state.time;
        state.item_pickup_blend_time = state.time;
        let item = *item_at(state.product, index)?;
        if item.item_type == ItemType::Weapon && self.host.options().autoswitch && item.tag != Weapon::Machinegun as i32
        {
            state.weapon_select_time = state.time;
            state.weapon_select = item.tag;
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn use_item(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
    ) -> PresentResult<()> {
        let es = event_entity(state, entity_ref)?.current_state.clone();
        let mut item = (es.event & !EV_EVENT_BITS) - EntityEvent::UseItem0 as i32;
        if item < 0 || item > Holdable::NumHoldable as i32 {
            item = 0;
        }
        if es.number == self.snapshot(state)?.player_state.client_num {
            let text = if item == 0 {
                "No item to use".to_owned()
            } else {
                format!(
                    "Use {}",
                    find_item_for_holdable(state.product, item)?.pickup_name.unwrap_or("")
                )
            };
            let y = self.center_y(state);
            self.host.center_print(&text, y, 16);
        }
        if item == Holdable::Teleporter as i32 {
            return Ok(());
        }
        if item == Holdable::Medkit as i32 {
            if es.client_num >= 0 && es.client_num < MAX_CLIENTS as i32 {
                let time = state.time;
                self.host.set_medkit_usage_time(static_state, es.client_num, time)?;
            }
            let sound = self.host.media().sounds.medkit_sound;
            self.host.start_sound(None, es.number, EV_BODY, sound);
            return Ok(());
        }
        if self.host.product() == Q3Product::MissionPack {
            if item == Holdable::Kamikaze as i32 || item == Holdable::Portal as i32 {
                return Ok(());
            }
            if item == Holdable::Invulnerability as i32 {
                let sound = self
                    .host
                    .mission_sounds()
                    .and_then(|sounds| sounds.use_invulnerability_sound);
                self.host.start_sound(None, es.number, EV_BODY, sound);
                return Ok(());
            }
        }
        let sound = self.host.media().sounds.use_nothing_sound;
        self.host.start_sound(None, es.number, EV_BODY, sound);
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn obituary(
        &mut self,
        state: &mut ClientGameState,
        static_state: &ClientGameStaticState,
        es: &EntityState,
    ) -> PresentResult<()> {
        let target = es.other_entity_num;
        let modification = es.event_parm;
        let mut attacker = es.other_entity_num2;
        if target < 0 || target >= MAX_CLIENTS as i32 {
            return Err(drop_msg("CG_Obituary: target out of range"));
        }
        let info = self.host.client_info_brief(static_state, target)?;
        let mut attacker_name = if attacker < 0 || attacker >= MAX_CLIENTS as i32 {
            attacker = ENTITYNUM_WORLD;
            None
        } else {
            self.host.player_name(attacker)
        };
        let Some(name) = self.host.player_name(target) else {
            return Ok(());
        };
        let target_name = format!("{}^7", name.chars().take(29).collect::<String>());
        let mut message: Option<String> = match modification {
            20 => Some("suicides".to_owned()),
            19 => Some("cratered".to_owned()),
            17 => Some("was squished".to_owned()),
            14 => Some("sank like a rock".to_owned()),
            15 => Some("melted".to_owned()),
            16 => Some("does a back flip into the lava".to_owned()),
            21 => Some("saw the light".to_owned()),
            22 => Some("was in the wrong place".to_owned()),
            _ => None,
        };
        if attacker == target {
            let ourselves = match info.gender {
                PlayerGender::Female => "herself",
                PlayerGender::Neuter => "itself",
                _ => "himself",
            };
            let possessive = match info.gender {
                PlayerGender::Female => "her",
                PlayerGender::Neuter => "its",
                _ => "his",
            };
            let mission = state.product == Q3Product::MissionPack;
            message = Some(if mission && modification == 26 {
                "goes out with a bang".to_owned()
            } else if modification == 5 {
                format!("tripped on {possessive} own grenade")
            } else if modification == 7 {
                format!("blew {ourselves} up")
            } else if modification == 9 {
                format!("melted {ourselves}")
            } else if modification == 13 {
                "should have used a smaller gun".to_owned()
            } else if mission && modification == 25 {
                let found = if info.gender == PlayerGender::Neuter {
                    "it's"
                } else {
                    possessive
                };
                format!("found {found} prox mine")
            } else {
                format!("killed {ourselves}")
            });
        }
        if let Some(message) = message {
            self.host.print(&format!("{target_name} {message}.\n"));
            return Ok(());
        }
        let ps = self.snapshot(state)?.player_state.clone();
        if attacker == ps.client_num {
            let mut text = format!("You fragged {target_name}");
            if self.host.options().game_type < GameType::Team {
                text += &game_format(
                    "\n%s place with %i",
                    &[
                        GameFormatArg::from(place_string(
                            ps.persistant.get(PersistentIndex::Rank as i32)?.wrapping_add(1),
                        )?),
                        GameFormatArg::from(ps.persistant.get(PersistentIndex::Score as i32)?),
                    ],
                    16_384,
                )?;
            }
            let mission = state.product == Q3Product::MissionPack;
            if !mission || !(self.host.options().single_player_active && self.host.options().camera_orbit) {
                let y = self.center_y(state);
                self.host.center_print(&text, y, 16);
            }
        }
        if attacker_name.is_none() {
            attacker = ENTITYNUM_WORLD;
            attacker_name = Some("noname".to_owned());
        } else {
            let name = attacker_name.expect("checked attacker name");
            let trimmed = format!("{}^7", name.chars().take(29).collect::<String>());
            if target == self.snapshot(state)?.player_state.client_num {
                state.killer_name = trimmed.clone();
            }
            attacker_name = Some(trimmed);
        }
        let mut suffix = "";
        if attacker != ENTITYNUM_WORLD {
            let mission = state.product == Q3Product::MissionPack;
            message = Some(match modification {
                2 => "was pummeled by".to_owned(),
                3 => "was machinegunned by".to_owned(),
                1 => "was gunned down by".to_owned(),
                4 => {
                    suffix = "'s grenade";
                    "ate".to_owned()
                }
                5 => {
                    suffix = "'s shrapnel";
                    "was shredded by".to_owned()
                }
                6 => {
                    suffix = "'s rocket";
                    "ate".to_owned()
                }
                7 => {
                    suffix = "'s rocket";
                    "almost dodged".to_owned()
                }
                8 | 9 => {
                    suffix = "'s plasmagun";
                    "was melted by".to_owned()
                }
                10 => "was railed by".to_owned(),
                11 => "was electrocuted by".to_owned(),
                12 | 13 => {
                    suffix = "'s BFG";
                    "was blasted by".to_owned()
                }
                18 => {
                    suffix = "'s personal space";
                    "tried to invade".to_owned()
                }
                _ => {
                    if modification == (if mission { 28 } else { 23 }) {
                        "was caught by".to_owned()
                    } else if mission && modification == 23 {
                        "was nailed by".to_owned()
                    } else if mission && modification == 24 {
                        suffix = "'s Chaingun";
                        "got lead poisoning from".to_owned()
                    } else if mission && modification == 25 {
                        suffix = "'s Prox Mine";
                        "was too close to".to_owned()
                    } else if mission && modification == 26 {
                        suffix = "'s Kamikaze blast";
                        "falls to".to_owned()
                    } else if mission && modification == 27 {
                        "was juiced by".to_owned()
                    } else {
                        "was killed by".to_owned()
                    }
                }
            });
            let attacker_name = attacker_name.expect("checked attacker name");
            self.host.print(&format!(
                "{target_name} {}{attacker_name}{suffix}\n",
                message.expect("obituary message")
            ));
            return Ok(());
        }
        self.host.print(&format!("{target_name} died.\n"));
        Ok(())
    }

    fn team_sound(
        &mut self,
        state: &mut ClientGameState,
        static_state: &ClientGameStaticState,
        event: i32,
    ) -> PresentResult<()> {
        let team = self.host.client_info_brief(static_state, state.client_num)?.team;
        match event {
            0 | 1 => {
                let expected = if event == 0 { Team::Red } else { Team::Blue };
                let sounds = self.host.media().sounds;
                self.host.add_buffered_sound(if team == expected {
                    sounds.capture_your_team_sound
                } else {
                    sounds.capture_opponent_sound
                });
            }
            2 | 3 => {
                let expected = if event == 2 { Team::Red } else { Team::Blue };
                let sounds = self.host.media().sounds;
                self.host.add_buffered_sound(if team == expected {
                    sounds.return_your_team_sound
                } else {
                    sounds.return_opponent_sound
                });
                self.host.add_buffered_sound(if event == 2 {
                    sounds.blue_flag_returned_sound
                } else {
                    sounds.red_flag_returned_sound
                });
            }
            4 | 5 => {
                let ps = self.snapshot(state)?.player_state.clone();
                let held = if event == 4 {
                    Powerup::BlueFlag
                } else {
                    Powerup::RedFlag
                };
                if ps.powerups.get(held as i32)? != 0 || ps.powerups.get(Powerup::NeutralFlag as i32)? != 0 {
                    return Ok(());
                }
                let threatened = if event == 4 { Team::Blue } else { Team::Red };
                if team != Team::Red && team != Team::Blue {
                    return Ok(());
                }
                if self.host.product() == Q3Product::MissionPack && self.host.options().game_type == GameType::OneFctf {
                    let sounds = self.host.mission_sounds().copied().unwrap_or_default();
                    self.host.add_buffered_sound(if team == threatened {
                        sounds.your_team_took_the_flag_sound
                    } else {
                        sounds.enemy_took_the_flag_sound
                    });
                } else {
                    let sounds = self.host.media().sounds;
                    self.host.add_buffered_sound(if team == threatened {
                        sounds.enemy_took_your_flag_sound
                    } else {
                        sounds.your_team_took_enemy_flag_sound
                    });
                }
            }
            6 | 7 => {
                let expected = if event == 6 { Team::Red } else { Team::Blue };
                if team == expected {
                    let sound = self.host.media().sounds.your_base_is_under_attack_sound;
                    self.host.add_buffered_sound(sound);
                }
            }
            8 => {
                let sound = self.host.media().sounds.red_scored_sound;
                self.host.add_buffered_sound(sound);
            }
            9 => {
                let sound = self.host.media().sounds.blue_scored_sound;
                self.host.add_buffered_sound(sound);
            }
            10 => {
                let sound = self.host.media().sounds.red_leads_sound;
                self.host.add_buffered_sound(sound);
            }
            11 => {
                let sound = self.host.media().sounds.blue_leads_sound;
                self.host.add_buffered_sound(sound);
            }
            12 => {
                let sound = self.host.media().sounds.teams_tied_sound;
                self.host.add_buffered_sound(sound);
            }
            13 if self.host.product() == Q3Product::MissionPack => {
                let sound = self.host.mission_sounds().and_then(|sounds| sounds.kamikaze_far_sound);
                self.host.start_local_sound(sound, EV_ANNOUNCER);
            }
            _ => {}
        }
        Ok(())
    }

    /// Dispatch an entity event through the recipe hook.
    pub fn entity_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        position: Vec3,
    ) -> PresentResult<()> {
        self.check_product(state)?;
        if self.host.pre_present_event(state, entity_ref, position) {
            self.source_entity_event(state, static_state, entity_ref, position)?;
            self.host.post_present_event(state, entity_ref, position);
        }
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn source_entity_event(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_ref: EventEntityRef,
        position: Vec3,
    ) -> PresentResult<()> {
        let es = event_entity(state, entity_ref)?.current_state.clone();
        let event = es.event & !EV_EVENT_BITS;
        if self.host.options().debug_events {
            let text = game_format(
                "ent:%3i  event:%3i ",
                &[GameFormatArg::from(es.number), GameFormatArg::from(event)],
                16_384,
            )?;
            self.host.print(&text);
        }
        if event == 0 {
            if self.host.options().debug_events {
                self.host.print("ZEROEVENT\n");
            }
            return Ok(());
        }
        let client_num = if es.client_num < 0 || es.client_num >= MAX_CLIENTS as i32 {
            0
        } else {
            es.client_num
        };
        let info = self.host.client_info_brief(static_state, client_num)?;
        let mission = self.host.product() == Q3Product::MissionPack;
        let mission_only = (event >= EntityEvent::ProximityMineStick as i32
            && event <= EntityEvent::LightningBolt as i32)
            || event >= EntityEvent::TauntYes as i32;
        let known = entity_event_name(event);
        if known.is_none()
            || event == EntityEvent::UseItem15 as i32
            || event == EntityEvent::Bullet as i32
            || (mission_only && !mission)
        {
            if self.host.options().debug_events {
                self.host.print("UNKNOWN\n");
            }
            return Err(drop_msg(format!("Unknown event: {event}")));
        }
        if self.host.options().debug_events {
            let label = if event >= EntityEvent::Step4 as i32 && event <= EntityEvent::Step16 as i32 {
                "EV_STEP"
            } else if event >= EntityEvent::Death1 as i32 && event <= EntityEvent::Death3 as i32 {
                "EV_DEATHx"
            } else {
                known.expect("checked event name")
            };
            self.host.print(&format!("{label}\n"));
        }
        if event >= EntityEvent::UseItem0 as i32 && event <= EntityEvent::UseItem14 as i32 {
            self.use_item(state, static_state, entity_ref)?;
            return Ok(());
        }
        let event_enum = EntityEvent::from_i32(event).expect("checked known event");
        match event_enum {
            EntityEvent::Footstep
            | EntityEvent::FootstepMetal
            | EntityEvent::Footsplash
            | EntityEvent::Footwade
            | EntityEvent::Swim => {
                if self.host.options().footsteps {
                    let pick = (self.host.rand_int() & 3) as usize;
                    let bank;
                    let sounds = if event_enum == EntityEvent::Footstep {
                        bank = *self.host.media().footsteps.bank(info.footsteps);
                        &bank
                    } else if event_enum == EntityEvent::FootstepMetal {
                        &self.host.media().footsteps.metal
                    } else {
                        &self.host.media().footsteps.splash
                    };
                    let sound = sounds[pick];
                    self.host.start_sound(None, es.number, EV_BODY, sound);
                }
            }
            EntityEvent::FallShort | EntityEvent::FallMedium | EntityEvent::FallFar => {
                if event_enum == EntityEvent::FallShort {
                    let sound = self.host.media().sounds.land_sound;
                    self.host.start_sound(None, es.number, EV_AUTO, sound);
                } else {
                    let (channel, name) = if event_enum == EntityEvent::FallMedium {
                        (EV_VOICE, "*pain100_1.wav")
                    } else {
                        (EV_AUTO, "*fall1.wav")
                    };
                    let sound = self.host.custom_sound(es.number, name);
                    self.host.start_sound(None, es.number, channel, sound);
                }
                if event_enum == EntityEvent::FallFar {
                    event_entity_mut(state, entity_ref)?.player.pain_time = state.time;
                }
                if client_num == state.predicted_player_state.client_num {
                    state.land_change = -8.0 * (event - EntityEvent::FallShort as i32 + 1) as f32;
                    state.land_time = state.time;
                }
            }
            EntityEvent::Step4 | EntityEvent::Step8 | EntityEvent::Step12 | EntityEvent::Step16 => {
                if client_num == state.predicted_player_state.client_num
                    && !self.host.options().demo_playback
                    && (self.snapshot(state)?.player_state.pm_flags & MoveFlags::FOLLOW) == 0
                    && !self.host.options().no_predict
                    && !self.host.options().synchronous_clients
                {
                    let delta = state.time.wrapping_sub(state.step_time);
                    let old_step = if delta < 200 {
                        state.step_change * (200 - delta) as f32 / 200.0
                    } else {
                        0.0
                    };
                    state.step_change = (old_step + 4.0 * (event - EntityEvent::Step4 as i32 + 1) as f32).min(32.0);
                    state.step_time = state.time;
                }
            }
            EntityEvent::JumpPad => {
                let shader = self.host.media().smoke_puff_shader.clone();
                self.host.smoke_puff(
                    state,
                    &SmokePuffDesc {
                        origin: event_entity(state, entity_ref)?.lerp_origin,
                        velocity: vec3(0.0, 0.0, 1.0),
                        radius: 32.0,
                        color: vec4(1.0, 1.0, 1.0, 0.33),
                        duration: 1000,
                        start_time: state.time,
                        fade_in_time: 0,
                        flags: 1,
                        shader,
                    },
                    None,
                );
                let sound = self.host.media().sounds.jump_pad_sound;
                let origin = event_entity(state, entity_ref)?.lerp_origin;
                self.host.start_sound(Some(origin), -1, EV_VOICE, sound);
                let sound = self.host.custom_sound(es.number, "*jump1.wav");
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::Jump => {
                let sound = self.host.custom_sound(es.number, "*jump1.wav");
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::Taunt => {
                let sound = self.host.custom_sound(es.number, "*taunt.wav");
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::TauntYes
            | EntityEvent::TauntNo
            | EntityEvent::TauntFollowMe
            | EntityEvent::TauntGetFlag
            | EntityEvent::TauntGuardBase
            | EntityEvent::TauntPatrol => {
                if mission {
                    let command = match event_enum {
                        EntityEvent::TauntYes => "yes",
                        EntityEvent::TauntNo => "no",
                        EntityEvent::TauntFollowMe => "followme",
                        EntityEvent::TauntGetFlag => "ongetflag",
                        EntityEvent::TauntGuardBase => "ondefense",
                        _ => "onpatrol",
                    };
                    self.host
                        .voice_chat_local(state, static_state, 1, false, es.number, 53, command)?;
                }
            }
            EntityEvent::WaterTouch => {
                let sound = self.host.media().sounds.watr_in_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::WaterLeave => {
                let sound = self.host.media().sounds.watr_out_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::WaterUnder => {
                let sound = self.host.media().sounds.watr_un_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::WaterClear => {
                let sound = self.host.custom_sound(es.number, "*gasp.wav");
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::ItemPickup | EntityEvent::GlobalItemPickup => {
                let index = es.event_parm;
                if index >= 1 && index < item_list(state.product).len() as i32 {
                    let item = *item_at(state.product, index)?;
                    if event_enum == EntityEvent::GlobalItemPickup {
                        if let Some(path) = item.pickup_sound {
                            let sound = self.host.register_sound(Some(path), false);
                            let client = self.snapshot(state)?.player_state.client_num;
                            self.host.start_sound(None, client, EV_AUTO, sound);
                        }
                    } else if item.item_type == ItemType::Powerup || item.item_type == ItemType::Team {
                        let sound = self.host.media().sounds.n_health_sound;
                        self.host.start_sound(None, es.number, EV_AUTO, sound);
                    } else if item.item_type == ItemType::PersistantPowerup {
                        if mission {
                            let sounds = self.host.mission_sounds().copied().unwrap_or_default();
                            let sound = if item.tag == Powerup::Scout as i32 {
                                sounds.scout_sound
                            } else if item.tag == Powerup::Guard as i32 {
                                sounds.guard_sound
                            } else if item.tag == Powerup::Doubler as i32 {
                                sounds.doubler_sound
                            } else if item.tag == Powerup::Ammoregen as i32 {
                                sounds.ammoregen_sound
                            } else {
                                None
                            };
                            if item.tag == Powerup::Scout as i32
                                || item.tag == Powerup::Guard as i32
                                || item.tag == Powerup::Doubler as i32
                                || item.tag == Powerup::Ammoregen as i32
                            {
                                self.host.start_sound(None, es.number, EV_AUTO, sound);
                            }
                        }
                    } else {
                        let sound = self.host.register_sound(item.pickup_sound, false);
                        self.host.start_sound(None, es.number, EV_AUTO, sound);
                    }
                    if es.number == self.snapshot(state)?.player_state.client_num {
                        self.item_pickup(state, index)?;
                    }
                }
            }
            EntityEvent::Noammo => {
                if es.number == self.snapshot(state)?.player_state.client_num {
                    self.host.out_of_ammo_change(state);
                }
            }
            EntityEvent::ChangeWeapon => {
                let sound = self.host.media().sounds.select_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::FireWeapon => {
                self.host.fire_weapon(state, entity_ref);
            }
            EntityEvent::PlayerTeleportIn | EntityEvent::PlayerTeleportOut => {
                let sounds = self.host.media().sounds;
                self.host.start_sound(
                    None,
                    es.number,
                    EV_AUTO,
                    if event_enum == EntityEvent::PlayerTeleportIn {
                        sounds.tele_in_sound
                    } else {
                        sounds.tele_out_sound
                    },
                );
                self.host.spawn_effect(state, position);
            }
            EntityEvent::ItemPop => {
                let sound = self.host.media().sounds.respawn_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::ItemRespawn => {
                let time = state.time;
                event_entity_mut(state, entity_ref)?.misc_time = time;
                let sound = self.host.media().sounds.respawn_sound;
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::GrenadeBounce => {
                let sounds = self.host.media().sounds;
                let sound = if (self.host.rand_int() & 1) != 0 {
                    sounds.hgrenb1a_sound
                } else {
                    sounds.hgrenb2a_sound
                };
                self.host.start_sound(None, es.number, EV_AUTO, sound);
            }
            EntityEvent::ProximityMineStick => {
                if mission {
                    let sounds = self.host.mission_sounds().copied().unwrap_or_default();
                    let sound = if (es.event_parm & 64) != 0 {
                        sounds.wstbimpl_sound
                    } else if (es.event_parm & 4096) != 0 {
                        sounds.wstbimpm_sound
                    } else {
                        sounds.wstbimpd_sound
                    };
                    self.host.start_sound(None, es.number, EV_AUTO, sound);
                }
            }
            EntityEvent::ProximityMineTrigger => {
                if mission {
                    let sound = self.host.mission_sounds().and_then(|sounds| sounds.wstbactv_sound);
                    self.host.start_sound(None, es.number, EV_AUTO, sound);
                }
            }
            EntityEvent::Kamikaze => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.kamikaze_effect(state, origin);
                }
            }
            EntityEvent::ObeliskExplode => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.obelisk_explode(state, origin);
                }
            }
            EntityEvent::ObeliskPain => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.obelisk_pain(state, origin);
                }
            }
            EntityEvent::InvulImpact => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.invulnerability_impact(state, origin, es.angles);
                }
            }
            EntityEvent::Juiced => {
                if mission {
                    let origin = event_entity(state, entity_ref)?.lerp_origin;
                    self.host.invulnerability_juiced(state, origin);
                }
            }
            EntityEvent::LightningBolt => {
                if mission {
                    self.host.lightning_bolt_beam(state, es.origin2, es.pos.base);
                }
            }
            EntityEvent::Scoreplum => {
                let origin = event_entity(state, entity_ref)?.lerp_origin;
                self.host.score_plum(state, es.other_entity_num, origin, es.time);
            }
            EntityEvent::MissileHit => {
                self.host.missile_hit_player(
                    state,
                    es.weapon,
                    position,
                    byte_to_direction(es.event_parm),
                    es.other_entity_num,
                );
            }
            EntityEvent::MissileMiss | EntityEvent::MissileMissMetal => {
                self.host.missile_hit_wall(
                    state,
                    es.weapon,
                    0,
                    position,
                    byte_to_direction(es.event_parm),
                    if event_enum == EntityEvent::MissileMiss {
                        ImpactSound::Default
                    } else {
                        ImpactSound::Metal
                    },
                );
            }
            EntityEvent::Railtrail => {
                event_entity_mut(state, entity_ref)?.current_state.weapon = Weapon::Railgun as i32;
                self.host.rail_trail(state, client_num, es.origin2, es.pos.base);
                if es.event_parm != 255 {
                    self.host.missile_hit_wall(
                        state,
                        Weapon::Railgun as i32,
                        es.client_num,
                        position,
                        byte_to_direction(es.event_parm),
                        ImpactSound::Default,
                    );
                }
            }
            EntityEvent::BulletHitWall => {
                self.host.bullet(
                    state,
                    es.pos.base,
                    es.other_entity_num,
                    BulletTarget::Wall {
                        normal: byte_to_direction(es.event_parm),
                    },
                );
            }
            EntityEvent::BulletHitFlesh => {
                self.host.bullet(
                    state,
                    es.pos.base,
                    es.other_entity_num,
                    BulletTarget::Flesh {
                        entity_num: es.event_parm,
                    },
                );
            }
            EntityEvent::Shotgun => {
                self.host.shotgun_fire(state, entity_ref);
            }
            EntityEvent::GeneralSound | EntityEvent::GlobalSound => {
                let sounds = self.host.media().game_sounds.clone();
                let sound = sounds
                    .get(es.event_parm as usize)
                    .copied()
                    .ok_or_else(|| range_msg(format!("Unregistered game sound index {}", es.event_parm)))?;
                if es.event_parm < 0 {
                    return Err(range_msg(format!("Unregistered game sound index {}", es.event_parm)));
                }
                let client = self.snapshot(state)?.player_state.client_num;
                let sound = match sound {
                    Some(sound) => Some(sound),
                    None => {
                        let path = self.host.sound_config_string(es.event_parm);
                        self.host.custom_sound(es.number, &path)
                    }
                };
                if event_enum == EntityEvent::GeneralSound {
                    self.host.start_sound(None, es.number, EV_VOICE, sound);
                } else {
                    self.host.start_sound(None, client, EV_AUTO, sound);
                }
            }
            EntityEvent::GlobalTeamSound => {
                self.team_sound(state, static_state, es.event_parm)?;
            }
            EntityEvent::Pain => {
                if es.number != self.snapshot(state)?.player_state.client_num {
                    self.pain_event(state, static_state, entity_ref, es.event_parm)?;
                }
            }
            EntityEvent::Death1 | EntityEvent::Death2 | EntityEvent::Death3 => {
                let sound = self.host.custom_sound(
                    es.number,
                    &format!("*death{}.wav", event - EntityEvent::Death1 as i32 + 1),
                );
                self.host.start_sound(None, es.number, EV_VOICE, sound);
            }
            EntityEvent::Obituary => {
                self.obituary(state, static_state, &es)?;
            }
            EntityEvent::PowerupQuad | EntityEvent::PowerupBattlesuit | EntityEvent::PowerupRegen => {
                if es.number == self.snapshot(state)?.player_state.client_num {
                    state.powerup_active = if event_enum == EntityEvent::PowerupQuad {
                        Powerup::Quad as i32
                    } else if event_enum == EntityEvent::PowerupBattlesuit {
                        Powerup::Battlesuit as i32
                    } else {
                        Powerup::Regen as i32
                    };
                    state.powerup_time = state.time;
                }
                let sounds = self.host.media().sounds;
                self.host.start_sound(
                    None,
                    es.number,
                    EV_ITEM,
                    if event_enum == EntityEvent::PowerupQuad {
                        sounds.quad_sound
                    } else if event_enum == EntityEvent::PowerupBattlesuit {
                        sounds.protect_sound
                    } else {
                        sounds.regen_sound
                    },
                );
            }
            EntityEvent::GibPlayer => {
                if (es.e_flags & 512) == 0 {
                    let sound = self.host.media().sounds.gib_sound;
                    self.host.start_sound(None, es.number, EV_BODY, sound);
                }
                let origin = event_entity(state, entity_ref)?.lerp_origin;
                self.host.gib_player(state, origin);
            }
            EntityEvent::StopLoopingSound => {
                self.host.stop_looping_sound(es.number);
                event_entity_mut(state, entity_ref)?.current_state.loop_sound = 0;
            }
            EntityEvent::DebugLine => {
                self.host.beam(state, entity_ref);
            }
            EntityEvent::None
            | EntityEvent::UseItem0
            | EntityEvent::UseItem1
            | EntityEvent::UseItem2
            | EntityEvent::UseItem3
            | EntityEvent::UseItem4
            | EntityEvent::UseItem5
            | EntityEvent::UseItem6
            | EntityEvent::UseItem7
            | EntityEvent::UseItem8
            | EntityEvent::UseItem9
            | EntityEvent::UseItem10
            | EntityEvent::UseItem11
            | EntityEvent::UseItem12
            | EntityEvent::UseItem13
            | EntityEvent::UseItem14
            | EntityEvent::UseItem15
            | EntityEvent::Bullet => {
                return Err(drop_msg(format!("Unknown event: {event}")));
            }
        }
        Ok(())
    }

    /// Check an entity's events (`checkEvents`).
    pub fn check_events(
        &mut self,
        state: &mut ClientGameState,
        static_state: &mut ClientGameStaticState,
        entity_number: usize,
    ) -> PresentResult<()> {
        self.check_product(state)?;
        let number = i32::try_from(entity_number).map_err(|_| range_msg("Entity number outside int32"))?;
        let entity = state.entity_at_mut(number)?;
        if entity.current_state.e_type > EntityType::Events as i32 {
            if entity.previous_event != 0 {
                return Ok(());
            }
            if (entity.current_state.e_flags & 16) != 0 {
                let other = entity.current_state.other_entity_num;
                entity.current_state.number = other;
            }
            entity.previous_event = 1;
            let event = entity.current_state.e_type - EntityType::Events as i32;
            entity.current_state.event = event;
        } else {
            if entity.current_state.event == entity.previous_event {
                return Ok(());
            }
            let event = entity.current_state.event;
            entity.previous_event = event;
            if (event & !EV_EVENT_BITS) == 0 {
                return Ok(());
            }
        }
        let server_time = self.snapshot(state)?.server_time;
        let entity = state.entity_at_mut(number)?;
        entity.lerp_origin = evaluate_trajectory(&entity.current_state.pos.clone(), server_time)?;
        self.host.set_entity_sound_position(state, entity_number);
        let position = state.entity_at(number)?.lerp_origin;
        self.entity_event(state, static_state, EventEntityRef::Entity(entity_number), position)
    }
}
