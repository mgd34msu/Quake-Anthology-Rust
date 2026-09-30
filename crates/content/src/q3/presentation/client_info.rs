//! Quake III presentation: client info.
//!
//! Donor provenance: `src/content/q3/presentation/client-info.ts`.

use crate::q3anim::{PlayerFootsteps, PlayerGender};
use qa_core::math::{vec3, Vec3};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::base::shared::definitions::*;
use crate::q3::foundation::animation_config::{Animation, MAX_ANIMATIONS_SENTINEL, TOTAL_ANIMATION_COUNT};
use crate::q3::presentation::hud::Shared;
use crate::q3::presentation::retail_snapshot::{default_model, PcmSound, SceneModel, SceneShader, SceneSkin};
use crate::q3::presentation::state::{ClientEntity, ClientGameState};

/// One stable `cgs.clientinfo` slot (`ClientInfo`).
#[derive(Debug, Clone, PartialEq)]
pub struct ClientInfo {
    /// Validity.
    pub info_valid: bool,
    /// Name.
    pub name: String,
    /// Team.
    pub team: Team,
    /// Bot skill.
    pub bot_skill: i32,
    /// Color 1.
    pub color1: Vec3,
    /// Color 2.
    pub color2: Vec3,
    /// Score.
    pub score: i32,
    /// Location.
    pub location: i32,
    /// Health.
    pub health: i32,
    /// Armor.
    pub armor: i32,
    /// Current weapon.
    pub cur_weapon: i32,
    /// Handicap.
    pub handicap: i32,
    /// Wins.
    pub wins: i32,
    /// Losses.
    pub losses: i32,
    /// Team task.
    pub team_task: i32,
    /// Team leader.
    pub team_leader: bool,
    /// Powerup bitmask.
    pub powerups: i32,
    /// Medkit usage time.
    pub medkit_usage_time: i32,
    /// Invulnerability start.
    pub invulnerability_start_time: i32,
    /// Invulnerability stop.
    pub invulnerability_stop_time: i32,
    /// Breath puff time.
    pub breath_puff_time: i32,
    /// Model name.
    pub model_name: String,
    /// Skin name.
    pub skin_name: String,
    /// Head model name.
    pub head_model_name: String,
    /// Head skin name.
    pub head_skin_name: String,
    /// Red team.
    pub red_team: String,
    /// Blue team.
    pub blue_team: String,
    /// Deferred.
    pub deferred: bool,
    /// New anims.
    pub new_anims: bool,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Head offset.
    pub head_offset: Vec3,
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Gender.
    pub gender: PlayerGender,
    /// Legs model.
    pub legs_model: SceneModel,
    /// Torso model.
    pub torso_model: SceneModel,
    /// Head model.
    pub head_model: SceneModel,
    /// Legs skin.
    pub legs_skin: Option<SceneSkin>,
    /// Torso skin.
    pub torso_skin: Option<SceneSkin>,
    /// Head skin.
    pub head_skin: Option<SceneSkin>,
    /// Model icon.
    pub model_icon: Option<SceneShader>,
    /// Animation cells (37; index 31 is the sentinel gap).
    pub animations: [Animation; TOTAL_ANIMATION_COUNT],
    /// Sounds (32).
    pub sounds: [Option<PcmSound>; 32],
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            info_valid: false,
            name: String::new(),
            team: Team::TeamFree,
            bot_skill: 0,
            color1: vec3(0.0, 0.0, 0.0),
            color2: vec3(0.0, 0.0, 0.0),
            score: 0,
            location: 0,
            health: 0,
            armor: 0,
            cur_weapon: 0,
            handicap: 0,
            wins: 0,
            losses: 0,
            team_task: 0,
            team_leader: false,
            powerups: 0,
            medkit_usage_time: 0,
            invulnerability_start_time: 0,
            invulnerability_stop_time: 0,
            breath_puff_time: 0,
            model_name: String::new(),
            skin_name: String::new(),
            head_model_name: String::new(),
            head_skin_name: String::new(),
            red_team: String::new(),
            blue_team: String::new(),
            deferred: false,
            new_anims: false,
            fixed_legs: false,
            fixed_torso: false,
            head_offset: vec3(0.0, 0.0, 0.0),
            footsteps: PlayerFootsteps::Normal,
            gender: PlayerGender::Male,
            legs_model: default_model(),
            torso_model: default_model(),
            head_model: default_model(),
            legs_skin: None,
            torso_skin: None,
            head_skin: None,
            model_icon: None,
            animations: [Animation::default(); TOTAL_ANIMATION_COUNT],
            sounds: std::array::from_fn(|_| None),
        }
    }
}

impl ClientInfo {
    /// Copy animation values into the stable cells (`setAnimations`).
    pub fn set_animations(&mut self, animations: &[Option<Animation>]) {
        if animations.len() != self.animations.len() {
            panic!("Client animation table has the wrong length");
        }
        for (index, source) in animations.iter().enumerate() {
            if source.is_none() && index == MAX_ANIMATIONS_SENTINEL {
                continue;
            }
            let Some(source) = source else {
                panic!("Missing client animation cell {index}");
            };
            self.animations[index] = *source;
        }
    }

    /// Copy all fields plus animation values (`copyFrom`).
    pub fn copy_from(&mut self, source: &ClientInfo) {
        *self = source.clone();
    }
}

/// Client info store (`ClientInfoStore`, used surface).
pub trait ClientInfoStore {
    /// Canonical frame state.
    fn state_handle(&self) -> Shared<ClientGameState>;
    /// Canonical client slot.
    fn client_info(&self, index: i32) -> Shared<ClientInfo>;
    /// Load deferred players.
    fn load_deferred_players(&mut self, reset: &mut dyn FnMut(&mut ClientEntity));
    /// Publish a client info string.
    fn new_client_info(&mut self, index: i32, config: &str);
    /// Reset the store.
    fn reset(&mut self);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::q3::foundation::animation_config::{Animation, MAX_ANIMATIONS_SENTINEL, TOTAL_ANIMATION_COUNT};

    #[test]
    fn client_info_animations_and_copy() {
        let mut info = ClientInfo::default();
        let mut rows = vec![None; TOTAL_ANIMATION_COUNT];
        for (index, row) in rows.iter_mut().enumerate() {
            if index != MAX_ANIMATIONS_SENTINEL {
                *row = Some(Animation {
                    first_frame: index as i32,
                    num_frames: 2,
                    ..Animation::default()
                });
            }
        }
        info.set_animations(&rows);
        assert_eq!(info.animations[0].first_frame, 0);
        assert_eq!(info.animations[MAX_ANIMATIONS_SENTINEL].first_frame, 0);
        info.name = "sarge".to_string();
        let mut other = ClientInfo::default();
        other.copy_from(&info);
        assert_eq!(other.name, "sarge");
        assert_eq!(other.animations[5].first_frame, 5);
    }

    #[test]
    #[should_panic(expected = "Client animation table has the wrong length")]
    fn client_info_animation_length() {
        ClientInfo::default().set_animations(&[None]);
    }
}
