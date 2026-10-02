//! QuakeC character reaction animations.
//!
//! Provenance: `src/app/bootstrap/simulation/quakec-character-animation.ts`.

use qa_content::q2::base::player::view::{q2_death_animation_frames, q2_pain_animation_frames};
use qa_content::q2::foundation::weapons::presentation::q2_attack_frames;
use qa_world::movement::types::{ActorAnimationState, AnimationState};

/// Reaction selecting a visual clip. Source events select visual clips only;
/// no damage, sound, or life callback is owned here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum QuakeCCharacterReaction {
    /// Attack.
    Attack,
    /// Pain.
    Pain,
    /// Death.
    Death,
    /// Alive (clear a death clip).
    Alive,
}

/// Select the reaction clip for a Quake II character, leaving other families
/// and higher-priority clips untouched.
#[must_use]
pub fn quake_c_character_animation(
    animation: &ActorAnimationState,
    reaction: QuakeCCharacterReaction,
    ducked: bool,
) -> ActorAnimationState {
    let AnimationState::Q2 { priority, run, .. } = animation.state else {
        return animation.clone();
    };
    if reaction == QuakeCCharacterReaction::Alive {
        if priority != 5 {
            return animation.clone();
        }
        return ActorAnimationState {
            provider: animation.provider.clone(),
            state: AnimationState::Q2 {
                frame: 0,
                end_frame: 39,
                priority: 0,
                duck: false,
                run: false,
            },
        };
    }
    if priority == 5 || (reaction == QuakeCCharacterReaction::Pain && priority >= 3) {
        return animation.clone();
    }
    let (first, last) = match reaction {
        QuakeCCharacterReaction::Attack => q2_attack_frames(ducked, 1),
        QuakeCCharacterReaction::Pain => q2_pain_animation_frames(ducked, 1),
        QuakeCCharacterReaction::Death => q2_death_animation_frames(ducked, 1),
        QuakeCCharacterReaction::Alive => unreachable!("handled above"),
    };
    ActorAnimationState {
        provider: animation.provider.clone(),
        state: AnimationState::Q2 {
            frame: first,
            end_frame: last,
            priority: match reaction {
                QuakeCCharacterReaction::Death => 5,
                QuakeCCharacterReaction::Pain => 3,
                _ => 4,
            },
            duck: ducked,
            run,
        },
    }
}

#[cfg(test)]
mod tests {
    use qa_core::identity::ProviderId;

    use super::*;

    fn animation(priority: i32) -> ActorAnimationState {
        ActorAnimationState {
            provider: ProviderId::new("test", "quakec"),
            state: AnimationState::Q2 {
                frame: 7,
                end_frame: 9,
                priority,
                duck: false,
                run: true,
            },
        }
    }

    #[test]
    fn ignores_non_q2_states() {
        let animation = ActorAnimationState {
            provider: ProviderId::new("test", "quakec"),
            state: AnimationState::Q1 {
                frame: 3,
                next_frame_seconds: 1.0,
            },
        };
        assert_eq!(
            quake_c_character_animation(&animation, QuakeCCharacterReaction::Death, false),
            animation
        );
    }

    #[test]
    fn attack_selects_frames_and_priority() {
        let out = quake_c_character_animation(&animation(0), QuakeCCharacterReaction::Attack, false);
        assert_eq!(
            out.state,
            AnimationState::Q2 {
                frame: 45,
                end_frame: 53,
                priority: 4,
                duck: false,
                run: true,
            }
        );
    }

    #[test]
    fn pain_yields_to_high_priority() {
        let before = animation(3);
        assert_eq!(
            quake_c_character_animation(&before, QuakeCCharacterReaction::Pain, false),
            before
        );
        let dead = animation(5);
        assert_eq!(
            quake_c_character_animation(&dead, QuakeCCharacterReaction::Attack, false),
            dead
        );
    }

    #[test]
    fn death_sets_priority_and_duck() {
        let out = quake_c_character_animation(&animation(0), QuakeCCharacterReaction::Death, true);
        let AnimationState::Q2 { priority, duck, .. } = out.state else {
            panic!("expected q2 state");
        };
        assert_eq!(priority, 5);
        assert!(duck);
    }

    #[test]
    fn alive_clears_death_clip_only() {
        let out = quake_c_character_animation(&animation(5), QuakeCCharacterReaction::Alive, false);
        assert_eq!(
            out.state,
            AnimationState::Q2 {
                frame: 0,
                end_frame: 39,
                priority: 0,
                duck: false,
                run: false,
            }
        );
        let before = animation(2);
        assert_eq!(
            quake_c_character_animation(&before, QuakeCCharacterReaction::Alive, false),
            before
        );
    }
}
