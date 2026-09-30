//! Quake III foundation: animation config.
//!
//! Donor provenance: `src/content/q3/foundation/animation-config.ts`.

use crate::q3anim::{PlayerFootsteps, PlayerGender};
use qa_core::math::{vec3, Vec3};
use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// animation-config.ts: CG_ParseAnimationFile.
// ---------------------------------------------------------------------------

pub(crate) const MAX_TEXT_BYTES: usize = 19_998;

pub(crate) const SOURCE_ANIMATION_COUNT: usize = 31;

pub(crate) const TOTAL_ANIMATION_COUNT: usize = 37;

pub(crate) const MAX_ANIMATIONS_SENTINEL: usize = 31;

/// One parsed animation row (`Animation`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Animation {
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

/// Retained client cells a parse writes into (`PlayerAnimationTarget`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationTarget {
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
    /// Animation cells.
    pub animations: [Animation; TOTAL_ANIMATION_COUNT],
}

impl Default for PlayerAnimationTarget {
    fn default() -> Self {
        Self {
            footsteps: PlayerFootsteps::Normal,
            head_offset: vec3(0.0, 0.0, 0.0),
            gender: PlayerGender::Male,
            fixed_legs: false,
            fixed_torso: false,
            animations: [Animation::default(); TOTAL_ANIMATION_COUNT],
        }
    }
}

/// Diagnostic warning with source position (`AnimationWarning`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationWarning {
    /// 1-based line.
    pub line: i32,
    /// 1-based column.
    pub column: i32,
    /// Message.
    pub message: String,
}

/// Parsed player animation config (`PlayerAnimationConfig`).
#[derive(Debug, Clone, PartialEq)]
pub struct PlayerAnimationConfig {
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
    /// Indexed by player animation; the `MAX_ANIMATIONS` sentinel is `None`.
    pub animations: [Option<Animation>; TOTAL_ANIMATION_COUNT],
    /// Diagnostics.
    pub warnings: Vec<AnimationWarning>,
}

pub(crate) fn animation_cell(
    target: &mut PlayerAnimationTarget,
    index: usize,
) -> Result<&mut Animation, Q3FoundationError> {
    target
        .animations
        .get_mut(index)
        .ok_or_else(|| range(format!("Missing client animation cell {index}")))
}

pub(crate) fn cg_print(
    format: &str,
    args: &[&str],
    print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<String, Q3FoundationError> {
    let mut position = 0;
    let mut message = String::new();
    let mut rest = format;
    while let Some(hit) = rest.find("%s") {
        message.push_str(&rest[..hit]);
        message.push_str(args.get(position).copied().unwrap_or(""));
        position += 1;
        rest = &rest[hit + 2..];
    }
    message.push_str(rest);
    if message.chars().count() >= 1024 {
        return Err(range("CG_Printf exceeds its 1024-byte source buffer"));
    }
    if let Some(print) = print {
        print(&message);
    }
    Ok(message)
}

/// Parse into retained client cells (`parsePlayerAnimationConfig` with context).
pub fn parse_player_animation_config_into(
    target: &mut PlayerAnimationTarget,
    parser: &mut CommonParseState,
    text: &str,
    source: &str,
    mut print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<PlayerAnimationConfig, Q3FoundationError> {
    let mut warnings: Vec<AnimationWarning> = Vec::new();
    if text.is_empty() {
        return Err(Q3FoundationError::Parse {
            source: source.to_string(),
            line: 1,
            column: 1,
            message: "empty animation file".to_string(),
        });
    }
    if text.len() > MAX_TEXT_BYTES {
        let message = cg_print("File %s too long\n", &[source], print.as_deref_mut())?;
        return Err(Q3FoundationError::Parse {
            source: source.to_string(),
            line: 1,
            column: 1,
            message,
        });
    }
    let mut cursor = CommonParseCursor::new(text)?;
    target.footsteps = PlayerFootsteps::Normal;
    target.head_offset = vec3(0.0, 0.0, 0.0);
    target.gender = PlayerGender::Male;
    target.fixed_legs = false;
    target.fixed_torso = false;

    loop {
        let previous = cursor.offset();
        let token = parser.parse(&mut cursor)?;
        let directive = token.to_lowercase();
        if directive == "footsteps" {
            let value = parser.parse(&mut cursor)?;
            match value.to_lowercase().as_str() {
                "default" | "normal" => target.footsteps = PlayerFootsteps::Normal,
                "boot" => target.footsteps = PlayerFootsteps::Boot,
                "flesh" => target.footsteps = PlayerFootsteps::Flesh,
                "mech" => target.footsteps = PlayerFootsteps::Mech,
                "energy" => target.footsteps = PlayerFootsteps::Energy,
                _ => {
                    let message = cg_print(
                        "Bad footsteps parm in %s: %s\n",
                        &[source, &value],
                        print.as_deref_mut(),
                    )?;
                    warnings.push(AnimationWarning {
                        line: parser.line().wrapping_add(1),
                        column: 1,
                        message,
                    });
                }
            }
        } else if directive == "headoffset" {
            let x = game_atof(&parser.parse(&mut cursor)?)?;
            let y = game_atof(&parser.parse(&mut cursor)?)?;
            let z = game_atof(&parser.parse(&mut cursor)?)?;
            target.head_offset = vec3(x, y, z);
        } else if directive == "sex" {
            let first = parser
                .parse(&mut cursor)?
                .chars()
                .next()
                .map(|c| c.to_lowercase().next().unwrap_or(c))
                .unwrap_or('\0');
            target.gender = if first == 'f' {
                PlayerGender::Female
            } else if first == 'n' {
                PlayerGender::Neuter
            } else {
                PlayerGender::Male
            };
        } else if directive == "fixedlegs" {
            target.fixed_legs = true;
        } else if directive == "fixedtorso" {
            target.fixed_torso = true;
        } else {
            let first = token.chars().next();
            if matches!(first, Some('0'..='9')) {
                cursor.set_offset(previous)?;
                break;
            }
            let message = cg_print("unknown token '%s' is %s\n", &[&token, source], print.as_deref_mut())?;
            warnings.push(AnimationWarning {
                line: parser.line().wrapping_add(1),
                column: 1,
                message,
            });
            if previous == cursor.offset() {
                return Err(range("CG animation prelude reached the source nonprogress cycle"));
            }
        }
    }

    let mut skip = 0i32;
    let mut index = 0usize;
    while index < SOURCE_ANIMATION_COUNT {
        let first_token = parser.parse(&mut cursor)?;
        if first_token.is_empty() {
            if (Q3PlayerAnimation::TORSO_GETFLAG as usize..=Q3PlayerAnimation::TORSO_NEGATIVE as usize).contains(&index)
            {
                let gesture = target.animations[Q3PlayerAnimation::TORSO_GESTURE as usize];
                let animation = animation_cell(target, index)?;
                animation.first_frame = gesture.first_frame;
                animation.frame_lerp = gesture.frame_lerp;
                animation.initial_lerp = gesture.initial_lerp;
                animation.loop_frames = gesture.loop_frames;
                animation.num_frames = gesture.num_frames;
                animation.reversed = false;
                animation.flipflop = false;
                index += 1;
                continue;
            }
            break;
        }
        let first_frame = game_atoi(&first_token)?;
        animation_cell(target, index)?.first_frame = first_frame;
        if index == Q3PlayerAnimation::LEGS_WALKCR as usize {
            let gesture = target.animations[Q3PlayerAnimation::TORSO_GESTURE as usize].first_frame;
            skip = target.animations[index].first_frame.wrapping_sub(gesture);
        }
        if (Q3PlayerAnimation::LEGS_WALKCR as usize..Q3PlayerAnimation::TORSO_GETFLAG as usize).contains(&index) {
            let animation = animation_cell(target, index)?;
            animation.first_frame = animation.first_frame.wrapping_sub(skip);
        }
        let count_token = parser.parse(&mut cursor)?;
        if count_token.is_empty() {
            break;
        }
        let num_frames = game_atoi(&count_token)?;
        let animation = animation_cell(target, index)?;
        animation.num_frames = num_frames;
        animation.reversed = false;
        animation.flipflop = false;
        if animation.num_frames < 0 {
            animation.num_frames = animation.num_frames.wrapping_neg();
            animation.reversed = true;
        }
        let loop_token = parser.parse(&mut cursor)?;
        if loop_token.is_empty() {
            break;
        }
        animation_cell(target, index)?.loop_frames = game_atoi(&loop_token)?;
        let fps_token = parser.parse(&mut cursor)?;
        if fps_token.is_empty() {
            break;
        }
        let mut fps = game_atof(&fps_token)?;
        if fps == 0.0 {
            fps = 1.0;
        }
        let lerp = qvm_float_to_int((1000.0f64 / f64::from(fps)) as f32);
        let animation = animation_cell(target, index)?;
        animation.frame_lerp = lerp;
        animation.initial_lerp = lerp;
        index += 1;
    }
    if index != SOURCE_ANIMATION_COUNT {
        let message = cg_print("Error parsing animation file: %s", &[source], print)?;
        return Err(Q3FoundationError::Parse {
            source: source.to_string(),
            line: parser.line().wrapping_add(1),
            column: 1,
            message,
        });
    }

    let walk_cr = target.animations[Q3PlayerAnimation::LEGS_WALKCR as usize];
    let back_cr = animation_cell(target, Q3PlayerAnimation::LEGS_BACKCR as usize)?;
    *back_cr = walk_cr;
    back_cr.reversed = true;
    let walk = target.animations[Q3PlayerAnimation::LEGS_WALK as usize];
    let back_walk = animation_cell(target, Q3PlayerAnimation::LEGS_BACKWALK as usize)?;
    *back_walk = walk;
    back_walk.reversed = true;
    let flag_run = animation_cell(target, Q3PlayerAnimation::FLAG_RUN as usize)?;
    flag_run.first_frame = 0;
    flag_run.num_frames = 16;
    flag_run.loop_frames = 16;
    flag_run.frame_lerp = 66;
    flag_run.initial_lerp = 66;
    flag_run.reversed = false;
    let flag_stand = animation_cell(target, Q3PlayerAnimation::FLAG_STAND as usize)?;
    flag_stand.first_frame = 16;
    flag_stand.num_frames = 5;
    flag_stand.loop_frames = 0;
    flag_stand.frame_lerp = 50;
    flag_stand.initial_lerp = 50;
    flag_stand.reversed = false;
    let flag_run2 = animation_cell(target, Q3PlayerAnimation::FLAG_STAND2RUN as usize)?;
    flag_run2.first_frame = 16;
    flag_run2.num_frames = 5;
    flag_run2.loop_frames = 1;
    flag_run2.frame_lerp = 66;
    flag_run2.initial_lerp = 66;
    flag_run2.reversed = true;

    let mut animations: [Option<Animation>; TOTAL_ANIMATION_COUNT] = [None; TOTAL_ANIMATION_COUNT];
    for (slot, cell) in animations.iter_mut().enumerate() {
        if slot != MAX_ANIMATIONS_SENTINEL {
            *cell = Some(target.animations[slot]);
        }
    }
    Ok(PlayerAnimationConfig {
        footsteps: target.footsteps,
        head_offset: target.head_offset,
        gender: target.gender,
        fixed_legs: target.fixed_legs,
        fixed_torso: target.fixed_torso,
        animations,
        warnings,
    })
}

/// Parse an owned config snapshot (`parsePlayerAnimationConfig` standalone).
pub fn parse_player_animation_config(text: &str, source: &str) -> Result<PlayerAnimationConfig, Q3FoundationError> {
    let mut target = PlayerAnimationTarget::default();
    let mut parser = CommonParseState::new();
    parse_player_animation_config_into(&mut target, &mut parser, text, source, None)
}
