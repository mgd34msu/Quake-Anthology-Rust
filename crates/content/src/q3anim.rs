//! Quake III player `animation.cfg` parser (`src/formats/q3-model/animation.ts`).
//!
//! Donor provenance: `src/formats/q3-model/animation.ts` (`bg_lib.c`,
//! `bg_pmove.c`). Footstep, gender, and head-offset parsing follows the
//! donor exactly; animation rows follow `CG_ParseAnimationFile`.

use qa_core::binary::BinaryError;
use qa_core::math::{vec3, Vec3};

use crate::model_text::ModelTokens;

/// Parsed player animation (`PlayerAnimation`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlayerAnimation {
    /// Animation name.
    pub name: String,
    /// First frame.
    pub first_frame: i32,
    /// Frame count.
    pub num_frames: i32,
    /// Loop frames.
    pub loop_frames: i32,
    /// Milliseconds per frame.
    pub frame_lerp: i32,
    /// Initial lerp.
    pub initial_lerp: i32,
    /// Reversed playback.
    pub reversed: bool,
    /// Flip-flop playback.
    pub flipflop: bool,
}

/// Player footsteps (`PlayerFootsteps`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerFootsteps {
    /// Normal steps.
    #[default]
    Normal,
    /// Boot steps.
    Boot,
    /// Flesh steps.
    Flesh,
    /// Mech steps.
    Mech,
    /// Energy steps.
    Energy,
}

/// Player gender (`PlayerGender`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlayerGender {
    /// Male.
    #[default]
    Male,
    /// Female.
    Female,
    /// Neuter.
    Neuter,
}

/// Parsed `animation.cfg` (`PlayerAnimationConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationConfig {
    /// Source name.
    pub source: String,
    /// Footsteps.
    pub footsteps: PlayerFootsteps,
    /// Head offset.
    pub head_offset: Vec3,
    /// Gender.
    pub gender: PlayerGender,
    /// Fixed legs.
    pub fixed_legs: bool,
    /// Fixed torso.
    pub fixed_torso: bool,
    /// Animations (37 slots; `MAX_ANIMATIONS` stays empty).
    pub animations: Vec<Option<PlayerAnimation>>,
    /// Loader diagnostics.
    pub diagnostics: Vec<String>,
}

/// Animation slot names (`Q3_ANIMATION_NAMES`).
pub const Q3_ANIMATION_NAMES: [&str; 37] = [
    "BOTH_DEATH1",
    "BOTH_DEAD1",
    "BOTH_DEATH2",
    "BOTH_DEAD2",
    "BOTH_DEATH3",
    "BOTH_DEAD3",
    "TORSO_GESTURE",
    "TORSO_ATTACK",
    "TORSO_ATTACK2",
    "TORSO_DROP",
    "TORSO_RAISE",
    "TORSO_STAND",
    "TORSO_STAND2",
    "LEGS_WALKCR",
    "LEGS_WALK",
    "LEGS_RUN",
    "LEGS_BACK",
    "LEGS_SWIM",
    "LEGS_JUMP",
    "LEGS_LAND",
    "LEGS_JUMPB",
    "LEGS_LANDB",
    "LEGS_IDLE",
    "LEGS_IDLECR",
    "LEGS_TURN",
    "TORSO_GETFLAG",
    "TORSO_GUARDBASE",
    "TORSO_PATROL",
    "TORSO_FOLLOWME",
    "TORSO_AFFIRMATIVE",
    "TORSO_NEGATIVE",
    "MAX_ANIMATIONS",
    "LEGS_BACKCR",
    "LEGS_BACKWALK",
    "FLAG_RUN",
    "FLAG_STAND",
    "FLAG_STAND2RUN",
];

/// Parse a player `animation.cfg` (`parsePlayerAnimationConfig`).
pub fn parse_player_animation_config(text: &str, source: &str) -> Result<PlayerAnimationConfig, BinaryError> {
    let mut tokens = ModelTokens::new(text, source);
    let mut footsteps = PlayerFootsteps::Normal;
    let mut head_offset = vec3(0.0, 0.0, 0.0);
    let mut gender = PlayerGender::Male;
    let mut fixed_legs = false;
    let mut fixed_torso = false;
    let mut animations: Vec<Option<PlayerAnimation>> = (0..37).map(|_| None).collect();
    let mut diagnostics = Vec::new();
    let mut gesture = PlayerAnimation {
        name: String::new(),
        first_frame: 0,
        num_frames: 6,
        loop_frames: 0,
        frame_lerp: 100,
        initial_lerp: 100,
        reversed: false,
        flipflop: false,
    };
    let mut first = tokens.next_token()?;
    while let Some(token) = first.clone() {
        if token.as_bytes().first().is_some_and(u8::is_ascii_digit) {
            break;
        }
        match token.to_ascii_lowercase().as_str() {
            "footsteps" => {
                let sounds = tokens.token()?.to_ascii_lowercase();
                footsteps = match sounds.as_str() {
                    "default" | "normal" => PlayerFootsteps::Normal,
                    "boot" => PlayerFootsteps::Boot,
                    "flesh" => PlayerFootsteps::Flesh,
                    "mech" => PlayerFootsteps::Mech,
                    "energy" => PlayerFootsteps::Energy,
                    _ => {
                        diagnostics.push(format!("Unknown footsteps {sounds}"));
                        PlayerFootsteps::Normal
                    }
                };
            }
            "sex" => {
                gender = match tokens.token()?.chars().next() {
                    Some('f') | Some('F') => PlayerGender::Female,
                    Some('n') | Some('N') => PlayerGender::Neuter,
                    _ => PlayerGender::Male,
                };
            }
            "headoffset" => {
                head_offset = vec3(tokens.float()?, tokens.float()?, tokens.float()?);
            }
            "fixedlegs" => fixed_legs = true,
            "fixedtorso" => fixed_torso = true,
            _ => diagnostics.push(format!("Unknown animation token {token}")),
        }
        first = tokens.next_token()?;
    }
    let mut leg_offset = 0;
    for index in 0..31 {
        let name = Q3_ANIMATION_NAMES[index];
        if index == 13 {
            let gesture_first = animations[6]
                .as_ref()
                .map_or(gesture.first_frame, |animation| animation.first_frame);
            let Some(current) = first.clone() else {
                return tokens.fail(format!("missing animation {name}"));
            };
            leg_offset = current.parse::<f64>().unwrap_or(f64::NAN) as i32 - gesture_first;
        }
        let Some(current) = first.clone() else {
            if index < 25 {
                return tokens.fail(format!("missing animation {name}"));
            }
            animations[index] = Some(PlayerAnimation {
                name: name.to_string(),
                num_frames: gesture.num_frames,
                ..gesture.clone()
            });
            continue;
        };
        let parsed: f64 = current.parse().unwrap_or(f64::NAN);
        if parsed.fract() != 0.0 || parsed < 0.0 {
            return tokens.fail(format!("invalid first frame {current}"));
        }
        let mut first_frame = parsed as i32;
        if (13..25).contains(&index) {
            first_frame -= leg_offset;
        }
        let num_frames = tokens.integer(-0x7fff_ffff, 0x7fff_ffff)?;
        let loop_frames = tokens.integer(-0x7fff_ffff, 0x7fff_ffff)?;
        let fps = tokens.float()?;
        let fps = if fps == 0.0 { 1.0 } else { fps };
        let frame_lerp = (1000.0 / f64::from(fps)) as f32;
        let frame_lerp = frame_lerp.trunc() as i32;
        gesture = PlayerAnimation {
            name: name.to_string(),
            first_frame,
            num_frames: num_frames.abs(),
            loop_frames,
            frame_lerp,
            initial_lerp: frame_lerp,
            reversed: num_frames < 0,
            flipflop: false,
        };
        animations[index] = Some(gesture.clone());
        if index < 30 {
            first = tokens.next_token()?;
        }
    }
    animations[32] = animations[13].clone().map(|animation| PlayerAnimation {
        name: Q3_ANIMATION_NAMES[32].to_string(),
        reversed: true,
        ..animation
    });
    animations[33] = animations[14].clone().map(|animation| PlayerAnimation {
        name: Q3_ANIMATION_NAMES[33].to_string(),
        reversed: true,
        ..animation
    });
    for (index, row) in [(34, (0, 16)), (35, (16, 5)), (36, (16, 5))] {
        animations[index] = Some(PlayerAnimation {
            name: Q3_ANIMATION_NAMES[index].to_string(),
            first_frame: row.0,
            num_frames: row.1,
            loop_frames: 0,
            frame_lerp: 1000 / 15,
            initial_lerp: 1000 / 15,
            reversed: false,
            flipflop: index == 35,
        });
    }
    Ok(PlayerAnimationConfig {
        source: source.to_string(),
        footsteps,
        head_offset,
        gender,
        fixed_legs,
        fixed_torso,
        animations,
        diagnostics,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> String {
        let mut text = String::from("sex f\nfootsteps boot\nheadoffset 1 2 3\nfixedlegs\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 1 0 10\n"));
        }
        text
    }

    #[test]
    fn q3_anim_round_trip() {
        let config = parse_player_animation_config(&fixture(), "<test>").unwrap();
        assert_eq!(config.gender, PlayerGender::Female);
        assert_eq!(config.footsteps, PlayerFootsteps::Boot);
        assert_eq!(config.head_offset, vec3(1.0, 2.0, 3.0));
        assert!(config.fixed_legs);
        assert!(!config.fixed_torso);
        assert!(config.diagnostics.is_empty());
        let death = config.animations[0].as_ref().unwrap();
        assert_eq!(death.name, "BOTH_DEATH1");
        assert_eq!(death.first_frame, 0);
        assert_eq!(death.frame_lerp, 100);
        // Leg rows subtract the crouch-walk offset (13 - (13 - 6)).
        let walk = config.animations[13].as_ref().unwrap();
        assert_eq!(walk.first_frame, 6);
        assert_eq!(config.animations[32].as_ref().unwrap().name, "LEGS_BACKCR");
        assert!(config.animations[32].as_ref().unwrap().reversed);
        assert!(config.animations[31].is_none());
        assert!(config.animations[35].as_ref().unwrap().flipflop);
    }

    #[test]
    fn q3_anim_rejects_bad_input() {
        let error = parse_player_animation_config("", "<test>").unwrap_err();
        assert!(error.message.contains("missing animation"), "{}", error.message);
        let error = parse_player_animation_config("0 1 0", "<test>").unwrap_err();
        assert!(error.message.contains("unexpected end"), "{}", error.message);
        // Unknown tokens are diagnostics, not errors.
        let mut text = String::from("oops\nfootsteps squeak\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 1 0 10\n"));
        }
        let config = parse_player_animation_config(&text, "<test>").unwrap();
        assert_eq!(config.diagnostics.len(), 2);
        let error = parse_player_animation_config("bogus\n", "<test>").unwrap_err();
        assert!(error.message.contains("missing animation"), "{}", error.message);
    }
}
