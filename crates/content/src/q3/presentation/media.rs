//! Quake III presentation: media.
//!
//! Donor provenance: `src/content/q3/presentation/media.ts`.

use qa_core::math::vec3;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::{GameType, Product, MAX_ITEMS};
use crate::q3::base::shared::items::{item_at, item_list};
use crate::q3::presentation::audio::PresentSound;
use crate::q3::presentation::character_resources::*;
use crate::q3::presentation::effects::*;
use crate::q3::presentation::entities::*;
use crate::q3::presentation::local_entities::*;
use crate::q3::presentation::model_access::*;
use crate::q3::presentation::ref_entity::*;
use crate::q3::presentation::ref_entity::{PresentError, PresentResult};
use crate::q3::presentation::weapons::*;

// ---------------------------------------------------------------------------
// media.ts
// ---------------------------------------------------------------------------

/// Client media sounds (`ClientMediaSounds`).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClientMediaSounds {
    /// One minute.
    pub one_minute_sound: Option<PresentSound>,
    /// Five minutes.
    pub five_minute_sound: Option<PresentSound>,
    /// Sudden death.
    pub sudden_death_sound: Option<PresentSound>,
    /// One frag.
    pub one_frag_sound: Option<PresentSound>,
    /// Two frags.
    pub two_frag_sound: Option<PresentSound>,
    /// Three frags.
    pub three_frag_sound: Option<PresentSound>,
    /// Count three.
    pub count3_sound: Option<PresentSound>,
    /// Count two.
    pub count2_sound: Option<PresentSound>,
    /// Count one.
    pub count1_sound: Option<PresentSound>,
    /// Fight.
    pub count_fight_sound: Option<PresentSound>,
    /// Prepare.
    pub count_prepare_sound: Option<PresentSound>,
    /// Prepare team.
    pub count_prepare_team_sound: Option<PresentSound>,
    /// Capture award.
    pub capture_award_sound: Option<PresentSound>,
    /// Red leads.
    pub red_leads_sound: Option<PresentSound>,
    /// Blue leads.
    pub blue_leads_sound: Option<PresentSound>,
    /// Teams tied.
    pub teams_tied_sound: Option<PresentSound>,
    /// Hit team.
    pub hit_team_sound: Option<PresentSound>,
    /// Red scored.
    pub red_scored_sound: Option<PresentSound>,
    /// Blue scored.
    pub blue_scored_sound: Option<PresentSound>,
    /// Capture your team.
    pub capture_your_team_sound: Option<PresentSound>,
    /// Capture opponent.
    pub capture_opponent_sound: Option<PresentSound>,
    /// Return your team.
    pub return_your_team_sound: Option<PresentSound>,
    /// Return opponent.
    pub return_opponent_sound: Option<PresentSound>,
    /// Taken your team.
    pub taken_your_team_sound: Option<PresentSound>,
    /// Taken opponent.
    pub taken_opponent_sound: Option<PresentSound>,
    /// Red flag returned.
    pub red_flag_returned_sound: Option<PresentSound>,
    /// Blue flag returned.
    pub blue_flag_returned_sound: Option<PresentSound>,
    /// Enemy took your flag.
    pub enemy_took_your_flag_sound: Option<PresentSound>,
    /// Your team took enemy flag.
    pub your_team_took_enemy_flag_sound: Option<PresentSound>,
    /// Neutral flag returned.
    pub neutral_flag_returned_sound: Option<PresentSound>,
    /// Your team took the flag.
    pub your_team_took_the_flag_sound: Option<PresentSound>,
    /// Enemy took the flag.
    pub enemy_took_the_flag_sound: Option<PresentSound>,
    /// You have flag.
    pub you_have_flag_sound: Option<PresentSound>,
    /// Holy shit.
    pub holy_shit_sound: Option<PresentSound>,
    /// Base under attack.
    pub your_base_is_under_attack_sound: Option<PresentSound>,
    /// Tracer.
    pub tracer_sound: Option<PresentSound>,
    /// Select.
    pub select_sound: Option<PresentSound>,
    /// Wear off.
    pub wear_off_sound: Option<PresentSound>,
    /// Use nothing.
    pub use_nothing_sound: Option<PresentSound>,
    /// Gib.
    pub gib_sound: Option<PresentSound>,
    /// Gib bounce 1.
    pub gib_bounce1_sound: Option<PresentSound>,
    /// Gib bounce 2.
    pub gib_bounce2_sound: Option<PresentSound>,
    /// Gib bounce 3.
    pub gib_bounce3_sound: Option<PresentSound>,
    /// Use invulnerability.
    pub use_invulnerability_sound: Option<PresentSound>,
    /// Invulnerability impact 1.
    pub invulnerability_impact_sound1: Option<PresentSound>,
    /// Invulnerability impact 2.
    pub invulnerability_impact_sound2: Option<PresentSound>,
    /// Invulnerability impact 3.
    pub invulnerability_impact_sound3: Option<PresentSound>,
    /// Invulnerability juiced.
    pub invulnerability_juiced_sound: Option<PresentSound>,
    /// Obelisk hit 1.
    pub obelisk_hit_sound1: Option<PresentSound>,
    /// Obelisk hit 2.
    pub obelisk_hit_sound2: Option<PresentSound>,
    /// Obelisk hit 3.
    pub obelisk_hit_sound3: Option<PresentSound>,
    /// Obelisk respawn.
    pub obelisk_respawn_sound: Option<PresentSound>,
    /// Ammo regen.
    pub ammoregen_sound: Option<PresentSound>,
    /// Doubler.
    pub doubler_sound: Option<PresentSound>,
    /// Guard.
    pub guard_sound: Option<PresentSound>,
    /// Scout.
    pub scout_sound: Option<PresentSound>,
    /// Teleport in.
    pub tele_in_sound: Option<PresentSound>,
    /// Teleport out.
    pub tele_out_sound: Option<PresentSound>,
    /// Respawn.
    pub respawn_sound: Option<PresentSound>,
    /// No ammo.
    pub no_ammo_sound: Option<PresentSound>,
    /// Talk.
    pub talk_sound: Option<PresentSound>,
    /// Land.
    pub land_sound: Option<PresentSound>,
    /// Hit.
    pub hit_sound: Option<PresentSound>,
    /// Hit high armor.
    pub hit_sound_high_armor: Option<PresentSound>,
    /// Hit low armor.
    pub hit_sound_low_armor: Option<PresentSound>,
    /// Impressive.
    pub impressive_sound: Option<PresentSound>,
    /// Excellent.
    pub excellent_sound: Option<PresentSound>,
    /// Denied.
    pub denied_sound: Option<PresentSound>,
    /// Humiliation.
    pub humiliation_sound: Option<PresentSound>,
    /// Assist.
    pub assist_sound: Option<PresentSound>,
    /// Defend.
    pub defend_sound: Option<PresentSound>,
    /// First impressive.
    pub first_impressive_sound: Option<PresentSound>,
    /// First excellent.
    pub first_excellent_sound: Option<PresentSound>,
    /// First humiliation.
    pub first_humiliation_sound: Option<PresentSound>,
    /// Taken lead.
    pub taken_lead_sound: Option<PresentSound>,
    /// Tied lead.
    pub tied_lead_sound: Option<PresentSound>,
    /// Lost lead.
    pub lost_lead_sound: Option<PresentSound>,
    /// Vote now.
    pub vote_now: Option<PresentSound>,
    /// Vote passed.
    pub vote_passed: Option<PresentSound>,
    /// Vote failed.
    pub vote_failed: Option<PresentSound>,
    /// Water in.
    pub watr_in_sound: Option<PresentSound>,
    /// Water out.
    pub watr_out_sound: Option<PresentSound>,
    /// Water under.
    pub watr_un_sound: Option<PresentSound>,
    /// Jump pad.
    pub jump_pad_sound: Option<PresentSound>,
    /// Flight.
    pub flight_sound: Option<PresentSound>,
    /// Medkit.
    pub medkit_sound: Option<PresentSound>,
    /// Quad.
    pub quad_sound: Option<PresentSound>,
    /// Ricochet 1.
    pub sfx_ric1: Option<PresentSound>,
    /// Ricochet 2.
    pub sfx_ric2: Option<PresentSound>,
    /// Ricochet 3.
    pub sfx_ric3: Option<PresentSound>,
    /// Railgun fire.
    pub sfx_railg: Option<PresentSound>,
    /// Rocket explosion.
    pub sfx_rockexp: Option<PresentSound>,
    /// Plasma explosion.
    pub sfx_plasmaexp: Option<PresentSound>,
    /// Prox explosion.
    pub sfx_proxexp: Option<PresentSound>,
    /// Nail hit.
    pub sfx_nghit: Option<PresentSound>,
    /// Nail hit flesh.
    pub sfx_nghitflesh: Option<PresentSound>,
    /// Nail hit metal.
    pub sfx_nghitmetal: Option<PresentSound>,
    /// Chaingun hit.
    pub sfx_chghit: Option<PresentSound>,
    /// Chaingun hit flesh.
    pub sfx_chghitflesh: Option<PresentSound>,
    /// Chaingun hit metal.
    pub sfx_chghitmetal: Option<PresentSound>,
    /// Weapon hover.
    pub weapon_hover_sound: Option<PresentSound>,
    /// Kamikaze explode.
    pub kamikaze_explode_sound: Option<PresentSound>,
    /// Kamikaze implode.
    pub kamikaze_implode_sound: Option<PresentSound>,
    /// Kamikaze far.
    pub kamikaze_far_sound: Option<PresentSound>,
    /// Winner.
    pub winner_sound: Option<PresentSound>,
    /// Loser.
    pub loser_sound: Option<PresentSound>,
    /// You suck.
    pub you_suck_sound: Option<PresentSound>,
    /// Prox impl.
    pub wstbimpl_sound: Option<PresentSound>,
    /// Prox impm.
    pub wstbimpm_sound: Option<PresentSound>,
    /// Prox impd.
    pub wstbimpd_sound: Option<PresentSound>,
    /// Prox actv.
    pub wstbactv_sound: Option<PresentSound>,
    /// Regen.
    pub regen_sound: Option<PresentSound>,
    /// Protect.
    pub protect_sound: Option<PresentSound>,
    /// N health.
    pub n_health_sound: Option<PresentSound>,
    /// Grenade bounce 1.
    pub hgrenb1a_sound: Option<PresentSound>,
    /// Grenade bounce 2.
    pub hgrenb2a_sound: Option<PresentSound>,
}

/// Client media graphics (`ClientMediaGraphics`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ClientMediaGraphics {
    /// Charset shader.
    pub charset_shader: Option<SceneShader>,
    /// White shader.
    pub white_shader: Option<SceneShader>,
    /// Charset prop.
    pub charset_prop: Option<SceneShader>,
    /// Charset prop glow.
    pub charset_prop_glow: Option<SceneShader>,
    /// Charset prop B.
    pub charset_prop_b: Option<SceneShader>,
    /// Number shaders.
    pub number_shaders: [Option<SceneShader>; 11],
    /// Bot skill shaders.
    pub bot_skill_shaders: [Option<SceneShader>; 5],
    /// View blood.
    pub view_blood_shader: Option<SceneShader>,
    /// Defer.
    pub defer_shader: Option<SceneShader>,
    /// Scoreboard name.
    pub scoreboard_name: Option<SceneShader>,
    /// Scoreboard ping.
    pub scoreboard_ping: Option<SceneShader>,
    /// Scoreboard score.
    pub scoreboard_score: Option<SceneShader>,
    /// Scoreboard time.
    pub scoreboard_time: Option<SceneShader>,
    /// Smoke puff.
    pub smoke_puff_shader: Option<SceneShader>,
    /// Smoke puff Rage Pro.
    pub smoke_puff_rage_pro_shader: Option<SceneShader>,
    /// Shotgun smoke puff.
    pub shotgun_smoke_puff_shader: Option<SceneShader>,
    /// Nail puff.
    pub nail_puff_shader: Option<SceneShader>,
    /// Blue prox mine.
    pub blue_prox_mine: SceneModel,
    /// Plasma ball.
    pub plasma_ball_shader: Option<SceneShader>,
    /// Blood trail.
    pub blood_trail_shader: Option<SceneShader>,
    /// Lagometer.
    pub lagometer_shader: Option<SceneShader>,
    /// Connection.
    pub connection_shader: Option<SceneShader>,
    /// Water bubble.
    pub water_bubble_shader: Option<SceneShader>,
    /// Tracer.
    pub tracer_shader: Option<SceneShader>,
    /// Select.
    pub select_shader: Option<SceneShader>,
    /// Crosshairs.
    pub crosshair_shader: [Option<SceneShader>; 10],
    /// Back tile.
    pub back_tile_shader: Option<SceneShader>,
    /// No ammo.
    pub noammo_shader: Option<SceneShader>,
    /// Quad.
    pub quad_shader: Option<SceneShader>,
    /// Quad weapon.
    pub quad_weapon_shader: Option<SceneShader>,
    /// Battle suit.
    pub battle_suit_shader: Option<SceneShader>,
    /// Battle weapon.
    pub battle_weapon_shader: Option<SceneShader>,
    /// Invisibility.
    pub invis_shader: Option<SceneShader>,
    /// Regen.
    pub regen_shader: Option<SceneShader>,
    /// Haste puff.
    pub haste_puff_shader: Option<SceneShader>,
    /// Red cube model.
    pub red_cube_model: SceneModel,
    /// Blue cube model.
    pub blue_cube_model: SceneModel,
    /// Red cube icon.
    pub red_cube_icon: Option<SceneShader>,
    /// Blue cube icon.
    pub blue_cube_icon: Option<SceneShader>,
    /// Red flag model.
    pub red_flag_model: SceneModel,
    /// Blue flag model.
    pub blue_flag_model: SceneModel,
    /// Red flag shaders.
    pub red_flag_shader: [Option<SceneShader>; 3],
    /// Blue flag shaders.
    pub blue_flag_shader: [Option<SceneShader>; 3],
    /// Flag pole model.
    pub flag_pole_model: SceneModel,
    /// Flag flap model.
    pub flag_flap_model: SceneModel,
    /// Red flag flap skin.
    pub red_flag_flap_skin: Option<SceneSkin>,
    /// Blue flag flap skin.
    pub blue_flag_flap_skin: Option<SceneSkin>,
    /// Neutral flag flap skin.
    pub neutral_flag_flap_skin: Option<SceneSkin>,
    /// Red flag base model.
    pub red_flag_base_model: SceneModel,
    /// Blue flag base model.
    pub blue_flag_base_model: SceneModel,
    /// Neutral flag base model.
    pub neutral_flag_base_model: SceneModel,
    /// Neutral flag model.
    pub neutral_flag_model: SceneModel,
    /// Flag shaders.
    pub flag_shader: [Option<SceneShader>; 4],
    /// Overload base model.
    pub overload_base_model: SceneModel,
    /// Overload target model.
    pub overload_target_model: SceneModel,
    /// Overload lights model.
    pub overload_lights_model: SceneModel,
    /// Overload energy model.
    pub overload_energy_model: SceneModel,
    /// Harvester model.
    pub harvester_model: SceneModel,
    /// Harvester red skin.
    pub harvester_red_skin: Option<SceneSkin>,
    /// Harvester blue skin.
    pub harvester_blue_skin: Option<SceneSkin>,
    /// Harvester neutral model.
    pub harvester_neutral_model: SceneModel,
    /// Red kamikaze shader.
    pub red_kamikaze_shader: Option<SceneShader>,
    /// Dust puff shader.
    pub dust_puff_shader: Option<SceneShader>,
    /// Friend shader.
    pub friend_shader: Option<SceneShader>,
    /// Red quad shader.
    pub red_quad_shader: Option<SceneShader>,
    /// Team status bar.
    pub team_status_bar: Option<SceneShader>,
    /// Blue kamikaze shader.
    pub blue_kamikaze_shader: Option<SceneShader>,
    /// Armor model.
    pub armor_model: SceneModel,
    /// Armor icon.
    pub armor_icon: Option<SceneShader>,
    /// Machinegun brass model.
    pub machinegun_brass_model: SceneModel,
    /// Shotgun brass model.
    pub shotgun_brass_model: SceneModel,
    /// Gib abdomen.
    pub gib_abdomen: SceneModel,
    /// Gib arm.
    pub gib_arm: SceneModel,
    /// Gib chest.
    pub gib_chest: SceneModel,
    /// Gib fist.
    pub gib_fist: SceneModel,
    /// Gib foot.
    pub gib_foot: SceneModel,
    /// Gib forearm.
    pub gib_forearm: SceneModel,
    /// Gib intestine.
    pub gib_intestine: SceneModel,
    /// Gib leg.
    pub gib_leg: SceneModel,
    /// Gib skull.
    pub gib_skull: SceneModel,
    /// Gib brain.
    pub gib_brain: SceneModel,
    /// Smoke 2.
    pub smoke2: SceneModel,
    /// Balloon shader.
    pub balloon_shader: Option<SceneShader>,
    /// Blood explosion shader.
    pub blood_explosion_shader: Option<SceneShader>,
    /// Bullet flash model.
    pub bullet_flash_model: SceneModel,
    /// Ring flash model.
    pub ring_flash_model: SceneModel,
    /// Dish flash model.
    pub dish_flash_model: SceneModel,
    /// Teleport effect model.
    pub teleport_effect_model: SceneModel,
    /// Teleport effect shader.
    pub teleport_effect_shader: Option<SceneShader>,
    /// Kamikaze effect model.
    pub kamikaze_effect_model: SceneModel,
    /// Kamikaze shock wave.
    pub kamikaze_shock_wave: SceneModel,
    /// Kamikaze head model.
    pub kamikaze_head_model: SceneModel,
    /// Kamikaze head trail.
    pub kamikaze_head_trail: SceneModel,
    /// Guard powerup model.
    pub guard_powerup_model: SceneModel,
    /// Scout powerup model.
    pub scout_powerup_model: SceneModel,
    /// Doubler powerup model.
    pub doubler_powerup_model: SceneModel,
    /// Ammo regen powerup model.
    pub ammo_regen_powerup_model: SceneModel,
    /// Invulnerability impact model.
    pub invulnerability_impact_model: SceneModel,
    /// Invulnerability juiced model.
    pub invulnerability_juiced_model: SceneModel,
    /// Medkit usage model.
    pub medkit_usage_model: SceneModel,
    /// Heart shader.
    pub heart_shader: Option<SceneShader>,
    /// Invulnerability powerup model.
    pub invulnerability_powerup_model: SceneModel,
    /// Medal impressive.
    pub medal_impressive: Option<SceneShader>,
    /// Medal excellent.
    pub medal_excellent: Option<SceneShader>,
    /// Medal gauntlet.
    pub medal_gauntlet: Option<SceneShader>,
    /// Medal defend.
    pub medal_defend: Option<SceneShader>,
    /// Medal assist.
    pub medal_assist: Option<SceneShader>,
    /// Medal capture.
    pub medal_capture: Option<SceneShader>,
    /// Bullet mark.
    pub bullet_mark_shader: Option<SceneShader>,
    /// Burn mark.
    pub burn_mark_shader: Option<SceneShader>,
    /// Hole mark.
    pub hole_mark_shader: Option<SceneShader>,
    /// Energy mark.
    pub energy_mark_shader: Option<SceneShader>,
    /// Shadow mark.
    pub shadow_mark_shader: Option<SceneShader>,
    /// Wake mark.
    pub wake_mark_shader: Option<SceneShader>,
    /// Blood mark.
    pub blood_mark_shader: Option<SceneShader>,
    /// Patrol.
    pub patrol_shader: Option<SceneShader>,
    /// Assault.
    pub assault_shader: Option<SceneShader>,
    /// Camp.
    pub camp_shader: Option<SceneShader>,
    /// Follow.
    pub follow_shader: Option<SceneShader>,
    /// Defend.
    pub defend_shader: Option<SceneShader>,
    /// Team leader.
    pub team_leader_shader: Option<SceneShader>,
    /// Retrieve.
    pub retrieve_shader: Option<SceneShader>,
    /// Escort.
    pub escort_shader: Option<SceneShader>,
    /// Cursor.
    pub cursor: Option<SceneShader>,
    /// Size cursor.
    pub size_cursor: Option<SceneShader>,
    /// Select cursor.
    pub select_cursor: Option<SceneShader>,
    /// Flag shaders.
    pub flag_shaders: [Option<SceneShader>; 3],
}

/// Footstep sound bank.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FootstepBank {
    /// Normal.
    pub normal: [Option<PresentSound>; 4],
    /// Boot.
    pub boot: [Option<PresentSound>; 4],
    /// Flesh.
    pub flesh: [Option<PresentSound>; 4],
    /// Mech.
    pub mech: [Option<PresentSound>; 4],
    /// Energy.
    pub energy: [Option<PresentSound>; 4],
    /// Splash.
    pub splash: [Option<PresentSound>; 4],
    /// Metal.
    pub metal: [Option<PresentSound>; 4],
}

impl FootstepBank {
    /// Bank by kind.
    pub fn get_mut(&mut self, kind: FootstepKind) -> &mut [Option<PresentSound>; 4] {
        match kind {
            FootstepKind::Normal => &mut self.normal,
            FootstepKind::Boot => &mut self.boot,
            FootstepKind::Flesh => &mut self.flesh,
            FootstepKind::Mech => &mut self.mech,
            FootstepKind::Energy => &mut self.energy,
            FootstepKind::Splash => &mut self.splash,
            FootstepKind::Metal => &mut self.metal,
        }
    }
}

/// Client game static state (`ClientGameStaticState`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientGameStaticState {
    /// Product.
    pub product: Product,
    /// Game type.
    pub game_type: GameType,
    /// Map name.
    pub mapname: String,
    /// Game models.
    pub game_models: Vec<SceneModel>,
    /// Game sounds.
    pub game_sounds: Vec<Option<PresentSound>>,
}

impl ClientGameStaticState {
    /// New static state.
    #[must_use]
    pub fn new(product: Product, game_type: GameType, mapname: impl Into<String>) -> Self {
        Self {
            product,
            game_type,
            mapname: mapname.into(),
            game_models: vec![default_model(); 256],
            game_sounds: vec![None; 256],
        }
    }
}

/// Client sound bank (`ClientSoundBank`, minimal mirror, synchronous).
pub trait ClientSoundBank {
    /// Register a sound.
    fn register_sound(&mut self, path: &str, compressed: bool) -> Option<PresentSound>;
}

/// Client media host (`ClientMediaHost`).
pub trait ClientMediaHost {
    /// Product.
    fn product(&self) -> Product;
    /// Game type.
    fn game_type(&self) -> GameType;
    /// Map name.
    fn mapname(&self) -> String;
    /// Client number.
    fn client_num(&self) -> i32;
    /// Config string.
    fn config_string(&self, index: usize) -> String;
    /// Build script.
    fn build_script(&self) -> bool;
    /// Loading string.
    fn loading_string(&mut self, text: &str);
    /// Loading item.
    fn loading_item(&mut self, index: usize);
    /// Loading client.
    fn loading_client(&mut self, index: usize);
    /// Clear the scene.
    fn clear_scene(&mut self);
    /// Reset the refdef.
    fn reset_refdef(&mut self);
    /// Load voice chats.
    fn load_voice_chats(&mut self);
    /// Build the spectator string.
    fn build_spectator_string(&mut self);
    /// New client info.
    fn new_client_info(&mut self, index: usize, info: &str);
}

/// Registered client graphics (`RegisteredClientGraphics`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredClientGraphics {
    /// World.
    pub world: PresentWorldScene,
    /// Particle animations.
    pub particle_animations: PresentParticleAnimations,
}

/// Client event media (`ClientEventMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientEventMedia {
    /// Sounds.
    pub sounds: ClientMediaSounds,
    /// Footsteps.
    pub footsteps: FootstepBank,
    /// Game sounds.
    pub game_sounds: Vec<Option<PresentSound>>,
    /// Smoke puff shader.
    pub smoke_puff_shader: Option<SceneShader>,
}

/// Player media (`PlayerMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerMedia {
    /// Graphics.
    pub graphics: ClientMediaGraphics,
    /// Flight sound.
    pub flight_sound: Option<PresentSound>,
}

/// Mission player media (`MissionPlayerMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq)]
pub struct MissionPlayerMedia {
    /// Graphics.
    pub graphics: ClientMediaGraphics,
}

/// Particle media (`ParticleMedia`, minimal mirror).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParticleMedia {
    /// Tracer shader.
    pub tracer_shader: Option<SceneShader>,
    /// Smoke puff shader.
    pub smoke_puff_shader: Option<SceneShader>,
    /// Water bubble shader.
    pub water_bubble_shader: Option<SceneShader>,
}

/// Client media (`ClientMedia`).
pub struct ClientMedia {
    /// Sounds.
    pub sounds: ClientMediaSounds,
    /// Graphics.
    pub graphics: ClientMediaGraphics,
    /// Weapon registry.
    pub weapon_registry: ClientWeaponMediaRegistry,
    /// Footsteps.
    pub footsteps: FootstepBank,
    /// Inline models.
    pub inline_models: Vec<InlineModelEntry>,
    /// Product.
    pub product: Product,
    /// Static state.
    pub static_state: ClientGameStaticState,
    /// Resources.
    pub resources: Box<dyn PresentRendererResources>,
    /// Sound bank.
    pub sound_bank: Box<dyn ClientSoundBank>,
}

impl ClientMedia {
    /// New media.
    pub fn new(
        product: Product,
        static_state: ClientGameStaticState,
        resources: Box<dyn PresentRendererResources>,
        sound_bank: Box<dyn ClientSoundBank>,
        weapon_audio: Box<dyn WeaponRegistrationAudio>,
        weapon_resources: Box<dyn PresentRendererResources>,
    ) -> PresentResult<Self> {
        if product != static_state.product {
            return Err(PresentError::state("Client media product differs from cgs"));
        }
        Ok(Self {
            sounds: ClientMediaSounds::default(),
            graphics: ClientMediaGraphics::default(),
            weapon_registry: ClientWeaponMediaRegistry::new(product, weapon_resources, weapon_audio),
            footsteps: FootstepBank::default(),
            inline_models: vec![InlineModelEntry {
                model: default_model(),
                midpoint: zero_vec3(),
            }],
            product,
            static_state,
            resources,
            sound_bank,
        })
    }

    /// Event media.
    #[must_use]
    pub fn events(&self) -> ClientEventMedia {
        ClientEventMedia {
            sounds: self.sounds.clone(),
            footsteps: self.footsteps.clone(),
            game_sounds: self.static_state.game_sounds.clone(),
            smoke_puff_shader: self.graphics.smoke_puff_shader.clone(),
        }
    }

    /// Player media.
    #[must_use]
    pub fn players(&self) -> PlayerMedia {
        PlayerMedia {
            graphics: self.graphics.clone(),
            flight_sound: self.sounds.flight_sound.clone(),
        }
    }

    /// Mission player media.
    pub fn mission_players(&self) -> PresentResult<MissionPlayerMedia> {
        if self.product != Product::Missionpack {
            return Err(PresentError::state("Mission player media requested in baseq3"));
        }
        Ok(MissionPlayerMedia {
            graphics: self.graphics.clone(),
        })
    }

    /// Packet media.
    #[must_use]
    pub fn packet(&self) -> PacketEntityMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        PacketEntityMedia {
            game_models: self.static_state.game_models.clone(),
            game_sounds: self.static_state.game_sounds.clone(),
            inline_models: self.inline_models.clone(),
            items: self.weapon_registry.items().to_vec(),
            weapons: self
                .weapon_registry
                .weapons()
                .iter()
                .map(|weapon| weapon.packet.clone())
                .collect(),
            plasma_ball_shader: graphics.plasma_ball_shader.clone(),
            red_flag_base_model: graphics.red_flag_base_model.clone(),
            blue_flag_base_model: graphics.blue_flag_base_model.clone(),
            neutral_flag_base_model: graphics.neutral_flag_base_model.clone(),
            variant: if self.product == Product::Baseq3 {
                PacketEntityMediaVariant::Base
            } else {
                PacketEntityMediaVariant::Mission(PacketMissionMedia {
                    weapon_hover_sound: sounds.weapon_hover_sound.clone(),
                    blue_prox_mine: graphics.blue_prox_mine.clone(),
                    overload_base_model: graphics.overload_base_model.clone(),
                    overload_energy_model: graphics.overload_energy_model.clone(),
                    overload_lights_model: graphics.overload_lights_model.clone(),
                    overload_target_model: graphics.overload_target_model.clone(),
                    obelisk_respawn_sound: sounds.obelisk_respawn_sound.clone(),
                    harvester_model: graphics.harvester_model.clone(),
                    harvester_neutral_model: graphics.harvester_neutral_model.clone(),
                    harvester_red_skin: graphics.harvester_red_skin.clone(),
                    harvester_blue_skin: graphics.harvester_blue_skin.clone(),
                })
            },
        }
    }

    /// Effect media.
    #[must_use]
    pub fn effects(&self) -> EffectMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        EffectMedia {
            water_bubble_shader: graphics.water_bubble_shader.clone(),
            smoke_puff_rage_pro_shader: graphics.smoke_puff_rage_pro_shader.clone(),
            blood_explosion_shader: graphics.blood_explosion_shader.clone(),
            teleport_effect_model: graphics.teleport_effect_model.clone(),
            gib_skull: graphics.gib_skull.clone(),
            gib_brain: graphics.gib_brain.clone(),
            gib_abdomen: graphics.gib_abdomen.clone(),
            gib_arm: graphics.gib_arm.clone(),
            gib_chest: graphics.gib_chest.clone(),
            gib_fist: graphics.gib_fist.clone(),
            gib_foot: graphics.gib_foot.clone(),
            gib_forearm: graphics.gib_forearm.clone(),
            gib_intestine: graphics.gib_intestine.clone(),
            gib_leg: graphics.gib_leg.clone(),
            smoke2: graphics.smoke2.clone(),
            variant: if self.product == Product::Baseq3 {
                EffectMediaVariant::Base {
                    teleport_effect_shader: graphics.teleport_effect_shader.clone(),
                }
            } else {
                EffectMediaVariant::Mission(MissionEffectMedia {
                    lightning_shader: self.weapon_registry.effects.lightning_shader.clone(),
                    kamikaze_effect_model: graphics.kamikaze_effect_model.clone(),
                    dish_flash_model: graphics.dish_flash_model.clone(),
                    rocket_explosion_shader: self.weapon_registry.effects.rocket_explosion_shader.clone(),
                    obelisk_hit_sounds: [
                        sounds.obelisk_hit_sound1.clone(),
                        sounds.obelisk_hit_sound2.clone(),
                        sounds.obelisk_hit_sound3.clone(),
                    ],
                    invulnerability_impact_model: graphics.invulnerability_impact_model.clone(),
                    invulnerability_impact_sounds: [
                        sounds.invulnerability_impact_sound1.clone(),
                        sounds.invulnerability_impact_sound2.clone(),
                        sounds.invulnerability_impact_sound3.clone(),
                    ],
                    invulnerability_juiced_model: graphics.invulnerability_juiced_model.clone(),
                    invulnerability_juiced_sound: sounds.invulnerability_juiced_sound.clone(),
                })
            },
        }
    }

    /// Particle media.
    #[must_use]
    pub fn particles(&self) -> ParticleMedia {
        ParticleMedia {
            tracer_shader: self.graphics.tracer_shader.clone(),
            smoke_puff_shader: self.graphics.smoke_puff_shader.clone(),
            water_bubble_shader: self.graphics.water_bubble_shader.clone(),
        }
    }

    /// Local entity media.
    #[must_use]
    pub fn local_entities(&self) -> LocalEntityHostMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        let base = LocalEntityMedia {
            blood_trail_shader: graphics.blood_trail_shader.clone(),
            blood_mark_shader: graphics.blood_mark_shader.clone(),
            burn_mark_shader: graphics.burn_mark_shader.clone(),
            number_shaders: graphics.number_shaders.to_vec(),
            gib_bounce_sounds: [
                sounds.gib_bounce1_sound.clone(),
                sounds.gib_bounce2_sound.clone(),
                sounds.gib_bounce3_sound.clone(),
            ],
        };
        if self.product == Product::Baseq3 {
            LocalEntityHostMedia::Base(base)
        } else {
            LocalEntityHostMedia::Mission(MissionLocalEntityMedia {
                base,
                kamikaze_shock_wave: graphics.kamikaze_shock_wave.clone(),
                kamikaze_explode_sound: sounds.kamikaze_explode_sound.clone(),
                kamikaze_implode_sound: sounds.kamikaze_implode_sound.clone(),
            })
        }
    }

    /// Weapon media.
    #[must_use]
    pub fn weapons(&self) -> WeaponPresentationMedia {
        let graphics = &self.graphics;
        let sounds = &self.sounds;
        WeaponPresentationMedia {
            models: WeaponPresentationModels {
                machinegun_brass: graphics.machinegun_brass_model.clone(),
                shotgun_brass: graphics.shotgun_brass_model.clone(),
                dish_flash: graphics.dish_flash_model.clone(),
                ring_flash: graphics.ring_flash_model.clone(),
                bullet_flash: graphics.bullet_flash_model.clone(),
            },
            shaders: WeaponPresentationShaders {
                smoke_puff: graphics.smoke_puff_shader.clone(),
                nail_puff: graphics.nail_puff_shader.clone(),
                shotgun_smoke_puff: graphics.shotgun_smoke_puff_shader.clone(),
                invis: graphics.invis_shader.clone(),
                battle_weapon: graphics.battle_weapon_shader.clone(),
                quad_weapon: graphics.quad_weapon_shader.clone(),
                select: graphics.select_shader.clone(),
                noammo: graphics.noammo_shader.clone(),
                hole_mark: graphics.hole_mark_shader.clone(),
                burn_mark: graphics.burn_mark_shader.clone(),
                energy_mark: graphics.energy_mark_shader.clone(),
                bullet_mark: graphics.bullet_mark_shader.clone(),
                tracer: graphics.tracer_shader.clone(),
            },
            sounds: WeaponPresentationSounds {
                quad: sounds.quad_sound.clone(),
                nail_hit_flesh: sounds.sfx_nghitflesh.clone(),
                nail_hit_metal: sounds.sfx_nghitmetal.clone(),
                nail_hit: sounds.sfx_nghit.clone(),
                prox_explosion: sounds.sfx_proxexp.clone(),
                rocket_explosion: sounds.sfx_rockexp.clone(),
                plasma_explosion: sounds.sfx_plasmaexp.clone(),
                chaingun_hit_flesh: sounds.sfx_chghitflesh.clone(),
                chaingun_hit_metal: sounds.sfx_chghitmetal.clone(),
                chaingun_hit: sounds.sfx_chghit.clone(),
                ricochet1: sounds.sfx_ric1.clone(),
                ricochet2: sounds.sfx_ric2.clone(),
                ricochet3: sounds.sfx_ric3.clone(),
                tracer: sounds.tracer_sound.clone(),
            },
        }
    }
}

pub(crate) fn validate_media(media: &ClientMedia, host: &dyn ClientMediaHost) -> PresentResult<()> {
    if host.product() != media.product {
        return Err(PresentError::state(
            "Client media registration requires its canonical cgame state",
        ));
    }
    Ok(())
}

pub(crate) fn item_bits(host: &dyn ClientMediaHost) -> PresentResult<String> {
    let bits = host.config_string(27);
    if bits.len() > MAX_ITEMS as usize {
        return Err(PresentError::range("CS_ITEMS exceeds source MAX_ITEMS precache buffer"));
    }
    Ok(bits)
}

/// Register item sounds (`registerItemSounds`).
pub fn register_item_sounds(media: &mut ClientMedia, number: i32) -> PresentResult<()> {
    let item = item_at(media.product, number)
        .ok()
        .ok_or_else(|| PresentError::drop(format!("Bad item index {number} on entity")))?;
    if let Some(pickup) = item.pickup_sound {
        media.sound_bank.register_sound(pickup, false);
    }
    let bytes = item.sounds.as_bytes();
    let mut offset = 0;
    while offset < bytes.len() {
        let start = offset;
        while offset < bytes.len() && bytes[offset] != b' ' {
            offset += 1;
        }
        let length = offset - start;
        if !(5..64).contains(&length) {
            return Err(PresentError::state(format!(
                "PrecacheItem: {} has bad precache string",
                item.class_name.unwrap_or("")
            )));
        }
        let name = &item.sounds[start..offset];
        if offset < bytes.len() {
            offset += 1;
        }
        if name.len() >= 3 && &name[name.len() - 3..] == "wav" {
            media.sound_bank.register_sound(name, false);
        }
    }
    Ok(())
}

/// Register loading graphics (`registerClientLoadingGraphics`).
pub fn register_client_loading_graphics(media: &mut ClientMedia) {
    media.graphics.charset_shader = media.resources.register_shader("gfx/2d/bigchars");
    media.graphics.white_shader = media.resources.register_shader("white");
    media.graphics.charset_prop = media.resources.register_shader_no_mip("menu/art/font1_prop.tga");
    media.graphics.charset_prop_glow = media.resources.register_shader_no_mip("menu/art/font1_prop_glo.tga");
    media.graphics.charset_prop_b = media.resources.register_shader_no_mip("menu/art/font2_prop.tga");
}

/// Register client sounds (`registerClientSounds`).
pub fn register_client_sounds(media: &mut ClientMedia, host: &mut dyn ClientMediaHost) -> PresentResult<()> {
    validate_media(media, host)?;
    let mission = media.product == Product::Missionpack;
    let game_type = media.static_state.game_type;
    if mission {
        host.load_voice_chats();
    }
    let bank = &mut media.sound_bank;
    let sounds = &mut media.sounds;
    sounds.one_minute_sound = bank.register_sound("sound/feedback/1_minute.wav", true);
    sounds.five_minute_sound = bank.register_sound("sound/feedback/5_minute.wav", true);
    sounds.sudden_death_sound = bank.register_sound("sound/feedback/sudden_death.wav", true);
    sounds.one_frag_sound = bank.register_sound("sound/feedback/1_frag.wav", true);
    sounds.two_frag_sound = bank.register_sound("sound/feedback/2_frags.wav", true);
    sounds.three_frag_sound = bank.register_sound("sound/feedback/3_frags.wav", true);
    sounds.count3_sound = bank.register_sound("sound/feedback/three.wav", true);
    sounds.count2_sound = bank.register_sound("sound/feedback/two.wav", true);
    sounds.count1_sound = bank.register_sound("sound/feedback/one.wav", true);
    sounds.count_fight_sound = bank.register_sound("sound/feedback/fight.wav", true);
    sounds.count_prepare_sound = bank.register_sound("sound/feedback/prepare.wav", true);
    if mission {
        sounds.count_prepare_team_sound = bank.register_sound("sound/feedback/prepare_team.wav", true);
    }
    if (game_type as i32) >= (GameType::GtTeam as i32) || host.build_script() {
        sounds.capture_award_sound = bank.register_sound("sound/teamplay/flagcapture_yourteam.wav", true);
        sounds.red_leads_sound = bank.register_sound("sound/feedback/redleads.wav", true);
        sounds.blue_leads_sound = bank.register_sound("sound/feedback/blueleads.wav", true);
        sounds.teams_tied_sound = bank.register_sound("sound/feedback/teamstied.wav", true);
        sounds.hit_team_sound = bank.register_sound("sound/feedback/hit_teammate.wav", true);
        sounds.red_scored_sound = bank.register_sound("sound/teamplay/voc_red_scores.wav", true);
        sounds.blue_scored_sound = bank.register_sound("sound/teamplay/voc_blue_scores.wav", true);
        sounds.capture_your_team_sound = bank.register_sound("sound/teamplay/flagcapture_yourteam.wav", true);
        sounds.capture_opponent_sound = bank.register_sound("sound/teamplay/flagcapture_opponent.wav", true);
        sounds.return_your_team_sound = bank.register_sound("sound/teamplay/flagreturn_yourteam.wav", true);
        sounds.return_opponent_sound = bank.register_sound("sound/teamplay/flagreturn_opponent.wav", true);
        sounds.taken_your_team_sound = bank.register_sound("sound/teamplay/flagtaken_yourteam.wav", true);
        sounds.taken_opponent_sound = bank.register_sound("sound/teamplay/flagtaken_opponent.wav", true);
        if game_type == GameType::GtCtf || host.build_script() {
            sounds.red_flag_returned_sound = bank.register_sound("sound/teamplay/voc_red_returned.wav", true);
            sounds.blue_flag_returned_sound = bank.register_sound("sound/teamplay/voc_blue_returned.wav", true);
            sounds.enemy_took_your_flag_sound = bank.register_sound("sound/teamplay/voc_enemy_flag.wav", true);
            sounds.your_team_took_enemy_flag_sound = bank.register_sound("sound/teamplay/voc_team_flag.wav", true);
        }
        if mission {
            if game_type == GameType::Gt1fctf || host.build_script() {
                sounds.neutral_flag_returned_sound =
                    bank.register_sound("sound/teamplay/flagreturn_opponent.wav", true);
                sounds.your_team_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_team_1flag.wav", true);
                sounds.enemy_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_enemy_1flag.wav", true);
            }
            if game_type == GameType::Gt1fctf || game_type == GameType::GtCtf || host.build_script() {
                sounds.you_have_flag_sound = bank.register_sound("sound/teamplay/voc_you_flag.wav", true);
                sounds.holy_shit_sound = bank.register_sound("sound/feedback/voc_holyshit.wav", true);
            }
            if game_type == GameType::GtObelisk || host.build_script() {
                sounds.your_base_is_under_attack_sound =
                    bank.register_sound("sound/teamplay/voc_base_attack.wav", true);
            }
        } else {
            sounds.you_have_flag_sound = bank.register_sound("sound/teamplay/voc_you_flag.wav", true);
            sounds.holy_shit_sound = bank.register_sound("sound/feedback/voc_holyshit.wav", true);
            sounds.neutral_flag_returned_sound = bank.register_sound("sound/teamplay/flagreturn_opponent.wav", true);
            sounds.your_team_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_team_1flag.wav", true);
            sounds.enemy_took_the_flag_sound = bank.register_sound("sound/teamplay/voc_enemy_1flag.wav", true);
        }
    }
    sounds.tracer_sound = bank.register_sound("sound/weapons/machinegun/buletby1.wav", false);
    sounds.select_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.select_sound, false);
    sounds.wear_off_sound = bank.register_sound("sound/items/wearoff.wav", false);
    sounds.use_nothing_sound = bank.register_sound("sound/items/use_nothing.wav", false);
    sounds.gib_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.gib_sound, false);
    sounds.gib_bounce1_sound = bank.register_sound("sound/player/gibimp1.wav", false);
    sounds.gib_bounce2_sound = bank.register_sound("sound/player/gibimp2.wav", false);
    sounds.gib_bounce3_sound = bank.register_sound("sound/player/gibimp3.wav", false);
    if mission {
        sounds.use_invulnerability_sound = bank.register_sound("sound/items/invul_activate.wav", false);
        sounds.invulnerability_impact_sound1 = bank.register_sound("sound/items/invul_impact_01.wav", false);
        sounds.invulnerability_impact_sound2 = bank.register_sound("sound/items/invul_impact_02.wav", false);
        sounds.invulnerability_impact_sound3 = bank.register_sound("sound/items/invul_impact_03.wav", false);
        sounds.invulnerability_juiced_sound = bank.register_sound("sound/items/invul_juiced.wav", false);
        sounds.obelisk_hit_sound1 = bank.register_sound("sound/items/obelisk_hit_01.wav", false);
        sounds.obelisk_hit_sound2 = bank.register_sound("sound/items/obelisk_hit_02.wav", false);
        sounds.obelisk_hit_sound3 = bank.register_sound("sound/items/obelisk_hit_03.wav", false);
        sounds.obelisk_respawn_sound = bank.register_sound("sound/items/obelisk_respawn.wav", false);
        sounds.ammoregen_sound = bank.register_sound("sound/items/cl_ammoregen.wav", false);
        sounds.doubler_sound = bank.register_sound("sound/items/cl_doubler.wav", false);
        sounds.guard_sound = bank.register_sound("sound/items/cl_guard.wav", false);
        sounds.scout_sound = bank.register_sound("sound/items/cl_scout.wav", false);
    }
    sounds.tele_in_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.tele_in_sound, false);
    sounds.tele_out_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.tele_out_sound, false);
    sounds.respawn_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.respawn_sound, false);
    sounds.no_ammo_sound = bank.register_sound("sound/weapons/noammo.wav", false);
    sounds.talk_sound = bank.register_sound("sound/player/talk.wav", false);
    sounds.land_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.land_sound, false);
    sounds.hit_sound = bank.register_sound("sound/feedback/hit.wav", false);
    if mission {
        sounds.hit_sound_high_armor = bank.register_sound("sound/feedback/hithi.wav", false);
        sounds.hit_sound_low_armor = bank.register_sound("sound/feedback/hitlo.wav", false);
    }
    sounds.impressive_sound = bank.register_sound("sound/feedback/impressive.wav", true);
    sounds.excellent_sound = bank.register_sound("sound/feedback/excellent.wav", true);
    sounds.denied_sound = bank.register_sound("sound/feedback/denied.wav", true);
    sounds.humiliation_sound = bank.register_sound("sound/feedback/humiliation.wav", true);
    sounds.assist_sound = bank.register_sound("sound/feedback/assist.wav", true);
    sounds.defend_sound = bank.register_sound("sound/feedback/defense.wav", true);
    if mission {
        sounds.first_impressive_sound = bank.register_sound("sound/feedback/first_impressive.wav", true);
        sounds.first_excellent_sound = bank.register_sound("sound/feedback/first_excellent.wav", true);
        sounds.first_humiliation_sound = bank.register_sound("sound/feedback/first_gauntlet.wav", true);
    }
    sounds.taken_lead_sound = bank.register_sound("sound/feedback/takenlead.wav", true);
    sounds.tied_lead_sound = bank.register_sound("sound/feedback/tiedlead.wav", true);
    sounds.lost_lead_sound = bank.register_sound("sound/feedback/lostlead.wav", true);
    if mission {
        sounds.vote_now = bank.register_sound("sound/feedback/vote_now.wav", true);
        sounds.vote_passed = bank.register_sound("sound/feedback/vote_passed.wav", true);
        sounds.vote_failed = bank.register_sound("sound/feedback/vote_failed.wav", true);
    }
    sounds.watr_in_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.watr_in_sound, false);
    sounds.watr_out_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.watr_out_sound, false);
    sounds.watr_un_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.watr_un_sound, false);
    sounds.jump_pad_sound = bank.register_sound(Q3_CHARACTER_SOUNDS.jump_pad_sound, false);

    for i in 0..4 {
        for (kind, name) in Q3_FOOTSTEP_PATHS {
            let path = format!("sound/player/footsteps/{}{}.wav", name, i + 1);
            media.footsteps.get_mut(kind)[i] = media.sound_bank.register_sound(&path, false);
        }
    }

    // Source copies CS_ITEMS, but its sound filtering condition is commented out.
    item_bits(host)?;
    let item_count = item_list(media.product).len();
    for i in 1..item_count {
        register_item_sounds(media, i as i32)?;
    }
    for i in 1..256 {
        let name = host.config_string(288 + i);
        if name.is_empty() {
            break;
        }
        if name.starts_with('*') {
            continue;
        }
        media.static_state.game_sounds[i] = media.sound_bank.register_sound(&name, false);
    }

    let bank = &mut media.sound_bank;
    let sounds = &mut media.sounds;
    sounds.flight_sound = bank.register_sound("sound/items/flight.wav", false);
    sounds.medkit_sound = bank.register_sound("sound/items/use_medkit.wav", false);
    sounds.quad_sound = bank.register_sound("sound/items/damage3.wav", false);
    sounds.sfx_ric1 = bank.register_sound("sound/weapons/machinegun/ric1.wav", false);
    sounds.sfx_ric2 = bank.register_sound("sound/weapons/machinegun/ric2.wav", false);
    sounds.sfx_ric3 = bank.register_sound("sound/weapons/machinegun/ric3.wav", false);
    sounds.sfx_railg = bank.register_sound("sound/weapons/railgun/railgf1a.wav", false);
    sounds.sfx_rockexp = bank.register_sound("sound/weapons/rocket/rocklx1a.wav", false);
    sounds.sfx_plasmaexp = bank.register_sound("sound/weapons/plasma/plasmx1a.wav", false);
    if mission {
        sounds.sfx_proxexp = bank.register_sound("sound/weapons/proxmine/wstbexpl.wav", false);
        sounds.sfx_nghit = bank.register_sound("sound/weapons/nailgun/wnalimpd.wav", false);
        sounds.sfx_nghitflesh = bank.register_sound("sound/weapons/nailgun/wnalimpl.wav", false);
        sounds.sfx_nghitmetal = bank.register_sound("sound/weapons/nailgun/wnalimpm.wav", false);
        sounds.sfx_chghit = bank.register_sound("sound/weapons/vulcan/wvulimpd.wav", false);
        sounds.sfx_chghitflesh = bank.register_sound("sound/weapons/vulcan/wvulimpl.wav", false);
        sounds.sfx_chghitmetal = bank.register_sound("sound/weapons/vulcan/wvulimpm.wav", false);
        sounds.weapon_hover_sound = bank.register_sound("sound/weapons/weapon_hover.wav", false);
        sounds.kamikaze_explode_sound = bank.register_sound("sound/items/kam_explode.wav", false);
        sounds.kamikaze_implode_sound = bank.register_sound("sound/items/kam_implode.wav", false);
        sounds.kamikaze_far_sound = bank.register_sound("sound/items/kam_explode_far.wav", false);
        sounds.winner_sound = bank.register_sound("sound/feedback/voc_youwin.wav", false);
        sounds.loser_sound = bank.register_sound("sound/feedback/voc_youlose.wav", false);
        sounds.you_suck_sound = bank.register_sound("sound/misc/yousuck.wav", false);
        sounds.wstbimpl_sound = bank.register_sound("sound/weapons/proxmine/wstbimpl.wav", false);
        sounds.wstbimpm_sound = bank.register_sound("sound/weapons/proxmine/wstbimpm.wav", false);
        sounds.wstbimpd_sound = bank.register_sound("sound/weapons/proxmine/wstbimpd.wav", false);
        sounds.wstbactv_sound = bank.register_sound("sound/weapons/proxmine/wstbactv.wav", false);
    }
    sounds.regen_sound = bank.register_sound("sound/items/regen.wav", false);
    sounds.protect_sound = bank.register_sound("sound/items/protect3.wav", false);
    sounds.n_health_sound = bank.register_sound("sound/items/n_health.wav", false);
    sounds.hgrenb1a_sound = bank.register_sound("sound/weapons/grenade/hgrenb1a.wav", false);
    sounds.hgrenb2a_sound = bank.register_sound("sound/weapons/grenade/hgrenb2a.wav", false);
    if mission {
        for name in [
            "sound/player/james/death1.wav",
            "sound/player/james/death2.wav",
            "sound/player/james/death3.wav",
            "sound/player/james/jump1.wav",
            "sound/player/james/pain25_1.wav",
            "sound/player/james/pain75_1.wav",
            "sound/player/james/pain100_1.wav",
            "sound/player/james/falling1.wav",
            "sound/player/james/gasp.wav",
            "sound/player/james/drown.wav",
            "sound/player/james/fall1.wav",
            "sound/player/james/taunt.wav",
            "sound/player/janet/death1.wav",
            "sound/player/janet/death2.wav",
            "sound/player/janet/death3.wav",
            "sound/player/janet/jump1.wav",
            "sound/player/janet/pain25_1.wav",
            "sound/player/janet/pain75_1.wav",
            "sound/player/janet/pain100_1.wav",
            "sound/player/janet/falling1.wav",
            "sound/player/janet/gasp.wav",
            "sound/player/janet/drown.wav",
            "sound/player/janet/fall1.wav",
            "sound/player/janet/taunt.wav",
        ] {
            bank.register_sound(name, false);
        }
    }
    Ok(())
}

/// Register client graphics (`registerClientGraphics`).
pub fn register_client_graphics(
    media: &mut ClientMedia,
    host: &mut dyn ClientMediaHost,
) -> PresentResult<RegisteredClientGraphics> {
    validate_media(media, host)?;
    let mission = media.product == Product::Missionpack;
    let game_type = media.static_state.game_type;
    host.reset_refdef();
    host.clear_scene();
    let mapname = host.mapname();
    host.loading_string(&mapname);
    let world = media.resources.load_world(&mapname);
    host.loading_string("game media");

    for (i, name) in [
        "zero", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "minus",
    ]
    .iter()
    .enumerate()
    {
        media.graphics.number_shaders[i] = media.resources.register_shader(&format!("gfx/2d/numbers/{name}_32b"));
    }
    for (i, name) in ["skill1", "skill2", "skill3", "skill4", "skill5"].iter().enumerate() {
        media.graphics.bot_skill_shaders[i] = media.resources.register_shader(&format!("menu/art/{name}.tga"));
    }
    media.graphics.view_blood_shader = media.resources.register_shader("viewBloodBlend");
    media.graphics.defer_shader = media.resources.register_shader_no_mip("gfx/2d/defer.tga");
    media.graphics.scoreboard_name = media.resources.register_shader_no_mip("menu/tab/name.tga");
    media.graphics.scoreboard_ping = media.resources.register_shader_no_mip("menu/tab/ping.tga");
    media.graphics.scoreboard_score = media.resources.register_shader_no_mip("menu/tab/score.tga");
    media.graphics.scoreboard_time = media.resources.register_shader_no_mip("menu/tab/time.tga");
    media.graphics.smoke_puff_shader = media.resources.register_shader("smokePuff");
    media.graphics.smoke_puff_rage_pro_shader = media.resources.register_shader("smokePuffRagePro");
    media.graphics.shotgun_smoke_puff_shader = media.resources.register_shader("shotgunSmokePuff");
    if mission {
        media.graphics.nail_puff_shader = media.resources.register_shader("nailtrail");
        media.graphics.blue_prox_mine = media.resources.register_model("models/weaphits/proxmineb.md3");
    }
    media.graphics.plasma_ball_shader = media.resources.register_shader("sprites/plasma1");
    media.graphics.blood_trail_shader = media.resources.register_shader("bloodTrail");
    media.graphics.lagometer_shader = media.resources.register_shader("lagometer");
    media.graphics.connection_shader = media.resources.register_shader("disconnected");
    media.graphics.water_bubble_shader = media.resources.register_shader("waterBubble");
    media.graphics.tracer_shader = media.resources.register_shader("gfx/misc/tracer");
    media.graphics.select_shader = media.resources.register_shader("gfx/2d/select");
    for i in 0..10 {
        let name = format!("gfx/2d/crosshair{}", (b'a' + i as u8) as char);
        media.graphics.crosshair_shader[i] = media.resources.register_shader(&name);
    }
    media.graphics.back_tile_shader = media.resources.register_shader("gfx/2d/backtile");
    media.graphics.noammo_shader = media.resources.register_shader("icons/noammo");
    media.graphics.quad_shader = media.resources.register_shader("powerups/quad");
    media.graphics.quad_weapon_shader = media.resources.register_shader("powerups/quadWeapon");
    media.graphics.battle_suit_shader = media.resources.register_shader("powerups/battleSuit");
    media.graphics.battle_weapon_shader = media.resources.register_shader("powerups/battleWeapon");
    media.graphics.invis_shader = media.resources.register_shader("powerups/invisibility");
    media.graphics.regen_shader = media.resources.register_shader("powerups/regen");
    media.graphics.haste_puff_shader = media.resources.register_shader("hasteSmokePuff");
    if game_type == GameType::GtCtf
        || mission && (game_type == GameType::Gt1fctf || game_type == GameType::GtHarvester)
        || host.build_script()
    {
        media.graphics.red_cube_model = media.resources.register_model("models/powerups/orb/r_orb.md3");
        media.graphics.blue_cube_model = media.resources.register_model("models/powerups/orb/b_orb.md3");
        media.graphics.red_cube_icon = media.resources.register_shader("icons/skull_red");
        media.graphics.blue_cube_icon = media.resources.register_shader("icons/skull_blue");
    }
    if game_type == GameType::GtCtf
        || mission && (game_type == GameType::Gt1fctf || game_type == GameType::GtHarvester)
        || host.build_script()
    {
        media.graphics.red_flag_model = media.resources.register_model("models/flags/r_flag.md3");
        media.graphics.blue_flag_model = media.resources.register_model("models/flags/b_flag.md3");
        media.graphics.red_flag_shader[0] = media.resources.register_shader_no_mip("icons/iconf_red1");
        media.graphics.red_flag_shader[1] = media.resources.register_shader_no_mip("icons/iconf_red2");
        media.graphics.red_flag_shader[2] = media.resources.register_shader_no_mip("icons/iconf_red3");
        media.graphics.blue_flag_shader[0] = media.resources.register_shader_no_mip("icons/iconf_blu1");
        media.graphics.blue_flag_shader[1] = media.resources.register_shader_no_mip("icons/iconf_blu2");
        media.graphics.blue_flag_shader[2] = media.resources.register_shader_no_mip("icons/iconf_blu3");
        if mission {
            media.graphics.flag_pole_model = media.resources.register_model("models/flag2/flagpole.md3");
            media.graphics.flag_flap_model = media.resources.register_model("models/flag2/flagflap3.md3");
            media.graphics.red_flag_flap_skin = media.resources.register_skin("models/flag2/red.skin");
            media.graphics.blue_flag_flap_skin = media.resources.register_skin("models/flag2/blue.skin");
            media.graphics.neutral_flag_flap_skin = media.resources.register_skin("models/flag2/white.skin");
            media.graphics.red_flag_base_model = media
                .resources
                .register_model("models/mapobjects/flagbase/red_base.md3");
            media.graphics.blue_flag_base_model = media
                .resources
                .register_model("models/mapobjects/flagbase/blue_base.md3");
            media.graphics.neutral_flag_base_model = media
                .resources
                .register_model("models/mapobjects/flagbase/ntrl_base.md3");
        }
    }
    if mission {
        if game_type == GameType::Gt1fctf || host.build_script() {
            media.graphics.neutral_flag_model = media.resources.register_model("models/flags/n_flag.md3");
            media.graphics.flag_shader[0] = media.resources.register_shader_no_mip("icons/iconf_neutral1");
            media.graphics.flag_shader[1] = media.resources.register_shader_no_mip("icons/iconf_red2");
            media.graphics.flag_shader[2] = media.resources.register_shader_no_mip("icons/iconf_blu2");
            media.graphics.flag_shader[3] = media.resources.register_shader_no_mip("icons/iconf_neutral3");
        }
        if game_type == GameType::GtObelisk || host.build_script() {
            media.graphics.overload_base_model = media.resources.register_model("models/powerups/overload_base.md3");
            media.graphics.overload_target_model =
                media.resources.register_model("models/powerups/overload_target.md3");
            media.graphics.overload_lights_model =
                media.resources.register_model("models/powerups/overload_lights.md3");
            media.graphics.overload_energy_model =
                media.resources.register_model("models/powerups/overload_energy.md3");
        }
        if game_type == GameType::GtHarvester || host.build_script() {
            media.graphics.harvester_model = media
                .resources
                .register_model("models/powerups/harvester/harvester.md3");
            media.graphics.harvester_red_skin = media.resources.register_skin("models/powerups/harvester/red.skin");
            media.graphics.harvester_blue_skin = media.resources.register_skin("models/powerups/harvester/blue.skin");
            media.graphics.harvester_neutral_model =
                media.resources.register_model("models/powerups/obelisk/obelisk.md3");
        }
        media.graphics.red_kamikaze_shader = media.resources.register_shader("models/weaphits/kamikred");
        media.graphics.dust_puff_shader = media.resources.register_shader("hasteSmokePuff");
    }
    if (game_type as i32) >= (GameType::GtTeam as i32) || host.build_script() {
        media.graphics.friend_shader = media.resources.register_shader("sprites/foe");
        media.graphics.red_quad_shader = media.resources.register_shader("powerups/blueflag");
        media.graphics.team_status_bar = media.resources.register_shader("gfx/2d/colorbar.tga");
        if mission {
            media.graphics.blue_kamikaze_shader = media.resources.register_shader("models/weaphits/kamikblu");
        }
    }
    media.graphics.armor_model = media.resources.register_model("models/powerups/armor/armor_yel.md3");
    media.graphics.armor_icon = media.resources.register_shader_no_mip("icons/iconr_yellow");
    media.graphics.machinegun_brass_model = media.resources.register_model("models/weapons2/shells/m_shell.md3");
    media.graphics.shotgun_brass_model = media.resources.register_model("models/weapons2/shells/s_shell.md3");
    media.graphics.gib_abdomen = media.resources.register_model("models/gibs/abdomen.md3");
    media.graphics.gib_arm = media.resources.register_model("models/gibs/arm.md3");
    media.graphics.gib_chest = media.resources.register_model("models/gibs/chest.md3");
    media.graphics.gib_fist = media.resources.register_model("models/gibs/fist.md3");
    media.graphics.gib_foot = media.resources.register_model("models/gibs/foot.md3");
    media.graphics.gib_forearm = media.resources.register_model("models/gibs/forearm.md3");
    media.graphics.gib_intestine = media.resources.register_model("models/gibs/intestine.md3");
    media.graphics.gib_leg = media.resources.register_model("models/gibs/leg.md3");
    media.graphics.gib_skull = media.resources.register_model("models/gibs/skull.md3");
    media.graphics.gib_brain = media.resources.register_model("models/gibs/brain.md3");
    media.graphics.smoke2 = media.resources.register_model("models/weapons2/shells/s_shell.md3");
    media.graphics.balloon_shader = media.resources.register_shader("sprites/balloon3");
    media.graphics.blood_explosion_shader = media.resources.register_shader("bloodExplosion");
    media.graphics.bullet_flash_model = media.resources.register_model("models/weaphits/bullet.md3");
    media.graphics.ring_flash_model = media.resources.register_model("models/weaphits/ring02.md3");
    media.graphics.dish_flash_model = media.resources.register_model("models/weaphits/boom01.md3");
    if mission {
        media.graphics.teleport_effect_model = media.resources.register_model("models/powerups/pop.md3");
    } else {
        media.graphics.teleport_effect_model = media.resources.register_model("models/misc/telep.md3");
        media.graphics.teleport_effect_shader = media.resources.register_shader("teleportEffect");
    }
    if mission {
        media.graphics.kamikaze_effect_model = media.resources.register_model("models/weaphits/kamboom2.md3");
        media.graphics.kamikaze_shock_wave = media.resources.register_model("models/weaphits/kamwave.md3");
        media.graphics.kamikaze_head_model = media.resources.register_model("models/powerups/kamikazi.md3");
        media.graphics.kamikaze_head_trail = media.resources.register_model("models/powerups/trailtest.md3");
        media.graphics.guard_powerup_model = media.resources.register_model("models/powerups/guard_player.md3");
        media.graphics.scout_powerup_model = media.resources.register_model("models/powerups/scout_player.md3");
        media.graphics.doubler_powerup_model = media.resources.register_model("models/powerups/doubler_player.md3");
        media.graphics.ammo_regen_powerup_model = media.resources.register_model("models/powerups/ammo_player.md3");
        media.graphics.invulnerability_impact_model =
            media.resources.register_model("models/powerups/shield/impact.md3");
        media.graphics.invulnerability_juiced_model =
            media.resources.register_model("models/powerups/shield/juicer.md3");
        media.graphics.medkit_usage_model = media.resources.register_model("models/powerups/regen.md3");
        media.graphics.heart_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/selectedhealth.tga");
    }
    media.graphics.invulnerability_powerup_model = media.resources.register_model("models/powerups/shield/shield.md3");
    media.graphics.medal_impressive = media.resources.register_shader_no_mip("medal_impressive");
    media.graphics.medal_excellent = media.resources.register_shader_no_mip("medal_excellent");
    media.graphics.medal_gauntlet = media.resources.register_shader_no_mip("medal_gauntlet");
    media.graphics.medal_defend = media.resources.register_shader_no_mip("medal_defend");
    media.graphics.medal_assist = media.resources.register_shader_no_mip("medal_assist");
    media.graphics.medal_capture = media.resources.register_shader_no_mip("medal_capture");

    let bits = item_bits(host)?;
    let bytes = bits.as_bytes();
    let item_count = item_list(media.product).len();
    for i in 1..item_count {
        if bytes.get(i) == Some(&b'1') || host.build_script() {
            host.loading_item(i);
            media.weapon_registry.register_item_visuals(i as i32)?;
        }
    }

    media.graphics.bullet_mark_shader = media.resources.register_shader("gfx/damage/bullet_mrk");
    media.graphics.burn_mark_shader = media.resources.register_shader("gfx/damage/burn_med_mrk");
    media.graphics.hole_mark_shader = media.resources.register_shader("gfx/damage/hole_lg_mrk");
    media.graphics.energy_mark_shader = media.resources.register_shader("gfx/damage/plasma_mrk");
    media.graphics.shadow_mark_shader = media.resources.register_shader("markShadow");
    media.graphics.wake_mark_shader = media.resources.register_shader("wake");
    media.graphics.blood_mark_shader = media.resources.register_shader("bloodMark");

    for i in 1..world.model_count {
        let model = media.resources.register_model(&format!("*{i}"));
        let bounds = model_bounds(&model);
        let midpoint = |min: f32, max: f32| min + 0.5 * (max - min);
        media.inline_models.push(InlineModelEntry {
            model,
            midpoint: vec3(
                midpoint(bounds.min.x, bounds.max.x),
                midpoint(bounds.min.y, bounds.max.y),
                midpoint(bounds.min.z, bounds.max.z),
            ),
        });
    }

    for i in 1..256 {
        let name = host.config_string(32 + i);
        if name.is_empty() {
            break;
        }
        media.static_state.game_models[i] = media.resources.register_model(&name);
    }
    if mission {
        media.graphics.patrol_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/patrol.tga");
        media.graphics.assault_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/assault.tga");
        media.graphics.camp_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/camp.tga");
        media.graphics.follow_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/follow.tga");
        media.graphics.defend_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/defend.tga");
        media.graphics.team_leader_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/team_leader.tga");
        media.graphics.retrieve_shader = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/retrieve.tga");
        media.graphics.escort_shader = media.resources.register_shader_no_mip("ui/assets/statusbar/escort.tga");
        media.graphics.cursor = media.resources.register_shader_no_mip("menu/art/3_cursor2");
        media.graphics.size_cursor = media.resources.register_shader_no_mip("ui/assets/sizecursor.tga");
        media.graphics.select_cursor = media.resources.register_shader_no_mip("ui/assets/selectcursor.tga");
        media.graphics.flag_shaders[0] = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/flag_in_base.tga");
        media.graphics.flag_shaders[1] = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/flag_capture.tga");
        media.graphics.flag_shaders[2] = media
            .resources
            .register_shader_no_mip("ui/assets/statusbar/flag_missing.tga");
        media.resources.register_model("models/players/james/lower.md3");
        media.resources.register_model("models/players/james/upper.md3");
        media.resources.register_model("models/players/heads/james/james.md3");
        media.resources.register_model("models/players/janet/lower.md3");
        media.resources.register_model("models/players/janet/upper.md3");
        media.resources.register_model("models/players/heads/janet/janet.md3");
    }
    let particle_animations = media.resources.load_particle_animations();
    Ok(RegisteredClientGraphics {
        world,
        particle_animations,
    })
}

/// Register clients (`registerClients`).
pub fn register_clients(media: &mut ClientMedia, host: &mut dyn ClientMediaHost) -> PresentResult<()> {
    validate_media(media, host)?;
    let client_num = host.client_num();
    host.loading_client(client_num.max(0) as usize);
    let info = host.config_string(544 + client_num.max(0) as usize);
    host.new_client_info(client_num.max(0) as usize, &info);
    for i in 0..64 {
        if i == client_num {
            continue;
        }
        let info = host.config_string(544 + i as usize);
        if info.is_empty() {
            continue;
        }
        host.loading_client(i as usize);
        host.new_client_info(i as usize, &info);
    }
    host.build_spectator_string();
    Ok(())
}
