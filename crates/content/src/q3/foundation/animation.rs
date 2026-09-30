//! Quake III foundation: animation.
//!
//! Donor provenance: `src/content/q3/foundation/animation.ts`.

use qa_core::numeric::qvm_float_to_int;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::animation_config::*;
use thiserror::Error;

// ---------------------------------------------------------------------------
// animation.ts: CG_SetLerpFrameAnimation, CG_RunLerpFrame, CG_ClearLerpFrame.
// ---------------------------------------------------------------------------

/// Lerp-frame failure (donor `CommonError("drop")`, `RangeError`, and
/// `Error` throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum AnimationError {
    /// Dropped animation selection (donor `CommonError` with `"drop"`).
    #[error("drop: {0}")]
    Drop(String),
    /// Out-of-range value (donor `RangeError`).
    #[error("{0}")]
    Range(String),
    /// Operation failure (donor `Error`).
    #[error("{0}")]
    Failed(String),
}

fn range(message: impl Into<String>) -> AnimationError {
    AnimationError::Range(message.into())
}

fn failed(message: impl Into<String>) -> AnimationError {
    AnimationError::Failed(message.into())
}

/// Animation toggle bit.
pub const ANIMATION_TOGGLE_BIT: i32 = 128;

pub(crate) const MAX_TOTAL_ANIMATIONS: usize = TOTAL_ANIMATION_COUNT;

/// Interpolated frame state (`LerpFrame`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LerpFrame {
    /// Previous frame.
    pub old_frame: i32,
    /// Previous frame time.
    pub old_frame_time: i32,
    /// Current frame.
    pub frame: i32,
    /// Current frame time.
    pub frame_time: i32,
    /// Blend factor.
    pub back_lerp: f32,
    /// Animation number with toggle bit.
    pub animation_number: i32,
    /// Current animation.
    pub current_animation: Option<Animation>,
    /// Animation start time.
    pub animation_time: i32,
}

/// Fresh lerp frame (`createLerpFrame`).
#[must_use]
pub fn create_lerp_frame() -> LerpFrame {
    LerpFrame {
        old_frame: 0,
        old_frame_time: 0,
        frame: 0,
        frame_time: 0,
        back_lerp: 0.0,
        animation_number: 0,
        current_animation: None,
        animation_time: 0,
    }
}

/// Lerp frame step input (`RunLerpFrameInput`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RunLerpFrameInput {
    /// Clock in milliseconds.
    pub time_ms: i32,
    /// Requested animation number.
    pub new_animation: i32,
    /// Speed scale (already `Math.fround` semantics: `f32` input is exact).
    pub speed_scale: f32,
    /// Freeze on frame zero.
    pub no_player_animations: bool,
}

pub(crate) fn animation_at(config: &PlayerAnimationConfig, index: i32) -> Result<Animation, AnimationError> {
    if index < 0 || index as usize >= MAX_TOTAL_ANIMATIONS {
        return Err(AnimationError::Drop(format!("Bad animation number: {index}")));
    }
    config.animations[index as usize].ok_or_else(|| range(format!("animation slot {index} is not playable")))
}

/// Select an animation and schedule its first frame (`setLerpFrameAnimation`).
pub fn set_lerp_frame_animation(
    config: &PlayerAnimationConfig,
    state: &mut LerpFrame,
    new_animation: i32,
    print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<(), AnimationError> {
    state.animation_number = new_animation;
    let animation = animation_at(config, new_animation & !ANIMATION_TOGGLE_BIT)?;
    state.current_animation = Some(animation);
    state.animation_time = state.frame_time.wrapping_add(animation.initial_lerp);
    if let Some(print) = print {
        print(&format!("Anim: {}\n", new_animation & !ANIMATION_TOGGLE_BIT));
    }
    Ok(())
}

pub(crate) fn current_animation(state: &LerpFrame) -> Result<Animation, AnimationError> {
    state
        .current_animation
        .ok_or_else(|| failed("lerp frame has no current animation"))
}

/// Advance one lerp-frame step (`runLerpFrame`).
pub fn run_lerp_frame(
    config: &PlayerAnimationConfig,
    state: &mut LerpFrame,
    input: &RunLerpFrameInput,
    mut print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<(), AnimationError> {
    if input.no_player_animations {
        state.old_frame = 0;
        state.frame = 0;
        state.back_lerp = 0.0;
        return Ok(());
    }
    if !input.speed_scale.is_finite() || input.speed_scale < 0.0 {
        return Err(range(format!(
            "animation speed scale {} must be finite and non-negative",
            input.speed_scale
        )));
    }
    if input.new_animation != state.animation_number || state.current_animation.is_none() {
        let sink = print.as_deref_mut();
        set_lerp_frame_animation(config, state, input.new_animation, sink)?;
    }
    if input.time_ms >= state.frame_time {
        state.old_frame = state.frame;
        state.old_frame_time = state.frame_time;
        let animation = current_animation(state)?;
        if animation.frame_lerp == 0 {
            return Ok(());
        }
        if input.time_ms < state.animation_time {
            state.frame_time = state.animation_time;
        } else {
            state.frame_time = state.old_frame_time.wrapping_add(animation.frame_lerp);
        }
        let diff = state.frame_time.wrapping_sub(state.animation_time);
        let truncated = (f64::from(diff) / f64::from(animation.frame_lerp)).trunc();
        let mut frame_offset = qvm_float_to_int(truncated as f32 * input.speed_scale);
        let mut frame_count = animation.num_frames;
        if animation.flipflop {
            frame_count = frame_count.wrapping_mul(2);
        }
        if frame_offset >= frame_count {
            frame_offset = frame_offset.wrapping_sub(frame_count);
            if animation.loop_frames != 0 {
                frame_offset = frame_offset.wrapping_rem(animation.loop_frames);
                frame_offset = frame_offset
                    .wrapping_add(animation.num_frames)
                    .wrapping_sub(animation.loop_frames);
            } else {
                frame_offset = frame_count.wrapping_sub(1);
                state.frame_time = input.time_ms;
            }
        }
        if animation.reversed {
            state.frame = animation
                .first_frame
                .wrapping_add(animation.num_frames)
                .wrapping_sub(1)
                .wrapping_sub(frame_offset);
        } else if animation.flipflop && frame_offset >= animation.num_frames {
            let flip = if animation.num_frames == 0 {
                0
            } else {
                frame_offset.wrapping_rem(animation.num_frames)
            };
            state.frame = animation
                .first_frame
                .wrapping_add(animation.num_frames)
                .wrapping_sub(1)
                .wrapping_sub(flip);
        } else {
            state.frame = animation.first_frame.wrapping_add(frame_offset);
        }
        if input.time_ms > state.frame_time {
            state.frame_time = input.time_ms;
            if let Some(print) = print.as_mut() {
                print("Clamp lf->frameTime\n");
            }
        }
    }
    if state.frame_time > input.time_ms.wrapping_add(200) {
        state.frame_time = input.time_ms;
    }
    if state.old_frame_time > input.time_ms {
        state.old_frame_time = input.time_ms;
    }
    if state.frame_time == state.old_frame_time {
        state.back_lerp = 0.0;
    } else {
        let elapsed = input.time_ms.wrapping_sub(state.old_frame_time) as f32;
        let duration = state.frame_time.wrapping_sub(state.old_frame_time) as f32;
        state.back_lerp = 1.0 - elapsed / duration;
    }
    Ok(())
}

/// Reset interpolation to an animation's first frame (`clearLerpFrame`).
pub fn clear_lerp_frame(
    config: &PlayerAnimationConfig,
    state: &mut LerpFrame,
    animation: i32,
    time_ms: i32,
    print: Option<&mut (dyn FnMut(&str) + '_)>,
) -> Result<(), AnimationError> {
    state.frame_time = time_ms;
    state.old_frame_time = time_ms;
    set_lerp_frame_animation(config, state, animation, print)?;
    let selected = current_animation(state)?;
    state.old_frame = selected.first_frame;
    state.frame = selected.first_frame;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn animation_fixture() -> String {
        let mut text = String::from("sex f\nfootsteps boot\nheadoffset 1 2 3\nfixedlegs\nfixedtorso\n");
        for frame in 0..31 {
            text.push_str(&format!("{frame} 6 0 10\n"));
        }
        text
    }

    #[test]
    fn lerp_frame_selects_advances_and_resets() {
        let config = parse_player_animation_config(&animation_fixture(), "<t>").unwrap();
        let mut state = create_lerp_frame();
        set_lerp_frame_animation(&config, &mut state, 129, None).unwrap();
        assert_eq!(state.animation_number, 129);
        assert_eq!(
            state.current_animation.unwrap().first_frame,
            config.animations[1].as_ref().unwrap().first_frame
        );
        assert!(set_lerp_frame_animation(&config, &mut state, 99, None).is_err());
        assert!(set_lerp_frame_animation(&config, &mut state, 31, None).is_err());

        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 500,
                new_animation: 129,
                speed_scale: 1.0,
                no_player_animations: false,
            },
            None,
        )
        .unwrap();
        assert!(state.frame >= state.old_frame);
        assert!((0.0..=1.0).contains(&state.back_lerp));

        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 600,
                new_animation: 129,
                speed_scale: 1.0,
                no_player_animations: true,
            },
            None,
        )
        .unwrap();
        assert_eq!((state.frame, state.back_lerp), (0, 0.0));

        assert!(run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 600,
                new_animation: 129,
                speed_scale: f32::NAN,
                no_player_animations: false,
            },
            None,
        )
        .is_err());

        clear_lerp_frame(&config, &mut state, 0, 700, None).unwrap();
        assert_eq!(state.frame, 0);
        assert_eq!(state.frame_time, 700);
    }

    #[test]
    fn lerp_frame_loops_and_reverses() {
        let mut config = parse_player_animation_config(&animation_fixture(), "<t>").unwrap();
        config.animations[0] = Some(Animation {
            first_frame: 10,
            num_frames: 4,
            loop_frames: 2,
            frame_lerp: 100,
            initial_lerp: 100,
            reversed: false,
            flipflop: false,
        });
        let mut state = create_lerp_frame();
        for time in [0, 100, 200, 300, 400, 500, 600] {
            run_lerp_frame(
                &config,
                &mut state,
                &RunLerpFrameInput {
                    time_ms: time,
                    new_animation: 0,
                    speed_scale: 1.0,
                    no_player_animations: false,
                },
                None,
            )
            .unwrap();
        }
        assert!((10..14).contains(&state.frame));

        config.animations[1] = Some(Animation {
            first_frame: 20,
            num_frames: 4,
            loop_frames: 0,
            frame_lerp: 100,
            initial_lerp: 100,
            reversed: true,
            flipflop: false,
        });
        let mut state = create_lerp_frame();
        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 0,
                new_animation: 1,
                speed_scale: 1.0,
                no_player_animations: false,
            },
            None,
        )
        .unwrap();
        let first = state.frame;
        run_lerp_frame(
            &config,
            &mut state,
            &RunLerpFrameInput {
                time_ms: 100,
                new_animation: 1,
                speed_scale: 1.0,
                no_player_animations: false,
            },
            None,
        )
        .unwrap();
        assert!(state.frame <= first);
    }
}
