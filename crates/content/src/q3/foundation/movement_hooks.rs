//! Quake III foundation: movement hooks.
//!
//! Donor provenance: `src/content/q3/foundation/movement-hooks.ts`.

use qa_core::identity::OwnedActor;
use qa_core::time::FrameContext;
use qa_world::movement::q3::animation::{run_q3_animation_operation, run_q3_torso_operation, Q3AnimationContext};
use qa_world::movement::q3::constants::{
    command_buttons, move_flags, move_type, player_animation, weapon, weapon_state,
};
use qa_world::movement::q3::types::{q3_weapon_state, Q3AnimationRequest, Q3AnimationStepResult, Q3Product};
use qa_world::movement::types::{ActorAnimationState, ArsenalState, MovementEffect, MovementEnvironment, UserCommand};
use thiserror::Error;

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::arsenal::ArsenalError;
use crate::q3::foundation::arsenal::*;

// ---------------------------------------------------------------------------
// movement-hooks.ts: PM_Firing, PM_Animate, PM_Weapon, PM_TorsoAnimation.
// ---------------------------------------------------------------------------

/// Movement hook failure (donor `RangeError`, `TypeError`, and `Error`
/// throws).
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum MovementHooksError {
    /// Out-of-range value (donor `RangeError`).
    #[error("{0}")]
    Range(String),
    /// Wrong provider or state kind (donor `TypeError`).
    #[error("{0}")]
    Type(String),
    /// Operation failure (donor `Error`).
    #[error("{0}")]
    Failed(String),
}

fn range(message: impl Into<String>) -> MovementHooksError {
    MovementHooksError::Range(message.into())
}

fn failed(message: impl Into<String>) -> MovementHooksError {
    MovementHooksError::Failed(message.into())
}

fn type_error(message: impl Into<String>) -> MovementHooksError {
    MovementHooksError::Type(message.into())
}

/// Hook execution mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Q3Execution {
    /// Authoritative.
    Authoritative,
    /// Prediction.
    Prediction,
}

/// Hook command fields (`Q3Command` fields the hooks read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3HookCommand {
    /// Buttons.
    pub buttons: i32,
    /// Requested weapon.
    pub weapon: i32,
}

/// Locomotion work state fields the hooks read (`Q3Motion` subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Q3MotionWork {
    /// Movement type.
    pub pm_type: i32,
    /// Movement flags.
    pub pm_flags: i32,
    /// Event sequence.
    pub event_sequence: i32,
    /// Product.
    pub product: Q3Product,
}

/// Hook input fields (`Q3MovementInput` subset).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3HookInput {
    /// Actor.
    pub actor: OwnedActor,
    /// Execution mode.
    pub execution: Q3Execution,
    /// User command, threaded into the arsenal step like the donor.
    pub command: UserCommand,
    /// Environment.
    pub environment: MovementEnvironment,
}

/// Movement hook context (`Q3HookContext`, foundation-read fields).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3HookContext {
    /// Input.
    pub input: Q3HookInput,
    /// Motion.
    pub motion: Q3MotionWork,
    /// Command.
    pub command: Q3HookCommand,
    /// Frame clock.
    pub frame: FrameContext,
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
}

/// Arsenal runtime storage (`Q3ArsenalRuntimeAccess`).
pub trait Q3ArsenalRuntimeAccess {
    /// Read runtime state.
    fn read(&self, actor: &OwnedActor, execution: Q3Execution) -> Q3ArsenalRuntimeState;
    /// Write runtime state.
    fn write(&self, actor: &OwnedActor, execution: Q3Execution, state: Q3ArsenalRuntimeState);
    /// Gauntlet contact test.
    fn gauntlet_hit(&self, context: &Q3HookContext) -> bool;
}

/// Derive arsenal controls from a command (`q3SourceArsenalControls`).
#[must_use]
pub fn q3_source_arsenal_controls(context: &Q3HookContext) -> Q3ArsenalControls {
    Q3ArsenalControls {
        attack: context.command.buttons & command_buttons::ATTACK != 0,
        use_holdable: context.command.buttons & command_buttons::USE_HOLDABLE != 0,
        requested_weapon: context.command.weapon,
    }
}

/// Weapon phase result (`Q3WeaponPhaseResult`).
#[derive(Debug, Clone, PartialEq)]
pub struct Q3WeaponPhaseResult {
    /// Arsenal.
    pub arsenal: ArsenalState,
    /// Animation.
    pub animation: ActorAnimationState,
    /// Effects.
    pub effects: Vec<MovementEffect>,
    /// Movement flags.
    pub movement_flags: i32,
}

/// Selected movement hooks (`Q3MovementHooks`). The animation and torso
/// stages report the donor's non-Q3 `TypeError` throw as
/// [`MovementHooksError::Type`].
pub trait Q3MovementHooks {
    /// Firing test.
    fn firing(&self, context: &Q3HookContext) -> bool;
    /// Animation request.
    fn animation(
        &self,
        request: &Q3AnimationRequest,
        context: &Q3HookContext,
    ) -> Result<Q3AnimationStepResult, MovementHooksError>;
    /// Weapon stage.
    fn weapon(&self, context: &Q3HookContext) -> Result<Q3WeaponPhaseResult, MovementHooksError>;
    /// Torso stage.
    fn torso(&self, context: &Q3HookContext) -> Result<Q3AnimationStepResult, MovementHooksError>;
}

pub(crate) fn hook_animation_context(context: &Q3HookContext) -> Q3AnimationContext {
    Q3AnimationContext {
        animation: context.animation.clone(),
        dead: context.motion.pm_type >= move_type::DEAD,
        elapsed_milliseconds: context.frame.elapsed.as_milliseconds_truncated(),
        buttons: context.command.buttons,
        product: context.motion.product,
        event_sequence: context.motion.event_sequence,
    }
}

/// Source Q3 movement hooks (`createQ3SourceMovementHooks`).
#[derive(Debug)]
pub struct Q3SourceMovementHooks<R> {
    runtime: R,
}

impl<R> Q3SourceMovementHooks<R> {
    /// Wrap runtime storage.
    pub fn new(runtime: R) -> Self {
        Self { runtime }
    }
}

/// Build source movement hooks (`createQ3SourceMovementHooks`).
pub fn create_q3_source_movement_hooks<R: Q3ArsenalRuntimeAccess>(runtime: R) -> Q3SourceMovementHooks<R> {
    Q3SourceMovementHooks::new(runtime)
}

impl<R: Q3ArsenalRuntimeAccess> Q3MovementHooks for Q3SourceMovementHooks<R> {
    fn firing(&self, context: &Q3HookContext) -> bool {
        let Some((source_weapon, _, _)) = q3_weapon_state(&context.arsenal) else {
            return false;
        };
        let Some(item) = q3_weapon_item(source_weapon) else {
            return false;
        };
        match item.ammo.as_deref() {
            None => true,
            Some(ammo) => {
                context
                    .arsenal
                    .ammo
                    .iter()
                    .find(|entry| entry.item == ammo)
                    .map_or(0.0, |entry| entry.count)
                    != 0.0
            }
        }
    }

    fn animation(
        &self,
        request: &Q3AnimationRequest,
        context: &Q3HookContext,
    ) -> Result<Q3AnimationStepResult, MovementHooksError> {
        run_q3_animation_operation(*request, &hook_animation_context(context))
            .map_err(|error| type_error(error.to_string()))
    }

    fn weapon(&self, context: &Q3HookContext) -> Result<Q3WeaponPhaseResult, MovementHooksError> {
        let previous = self.runtime.read(&context.input.actor, context.input.execution);
        let state = Q3ArsenalRuntimeState {
            respawned: context.motion.pm_flags & move_flags::RESPAWNED != 0,
            use_item_held: context.motion.pm_flags & move_flags::USE_ITEM_HELD != 0,
            event_sequence: context.motion.event_sequence,
            ..previous
        };
        let result = step_q3_arsenal(
            &crate::q3::foundation::arsenal::WeaponStepInput {
                actor: context.input.actor.clone(),
                command: context.input.command,
                frame: context.frame,
                arsenal: context.arsenal.clone(),
                animation: context.animation.clone(),
                environment: context.input.environment,
                gauntlet_hit: self.runtime.gauntlet_hit(context),
            },
            &state,
            &q3_source_arsenal_controls(context),
            None,
        )
        .map_err(|error| match error {
            ArsenalError::BadClock => range(error.to_string()),
            ArsenalError::NotQ3Arsenal => type_error(error.to_string()),
            _ => failed(error.to_string()),
        })?;
        self.runtime
            .write(&context.input.actor, context.input.execution, result.runtime.clone());
        let movement_flags = (context.motion.pm_flags & !(move_flags::RESPAWNED | move_flags::USE_ITEM_HELD))
            | (if result.runtime.respawned {
                move_flags::RESPAWNED
            } else {
                0
            })
            | (if result.runtime.use_item_held {
                move_flags::USE_ITEM_HELD
            } else {
                0
            });
        Ok(Q3WeaponPhaseResult {
            arsenal: result.arsenal,
            animation: result.animation,
            effects: result.effects,
            movement_flags,
        })
    }

    fn torso(&self, context: &Q3HookContext) -> Result<Q3AnimationStepResult, MovementHooksError> {
        let Some((source_weapon, state, _)) = q3_weapon_state(&context.arsenal) else {
            return Err(type_error("Q3 source hooks require Q3 weapon state"));
        };
        if state != weapon_state::READY {
            return Ok(Q3AnimationStepResult {
                animation: context.animation.clone(),
                effects: Vec::new(),
            });
        }
        let animation = if source_weapon == weapon::GAUNTLET {
            player_animation::TORSO_STAND2
        } else {
            player_animation::TORSO_STAND
        };
        run_q3_torso_operation(animation, &hook_animation_context(context), true)
            .map_err(|error| type_error(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use crate::q3::base::shared::definitions::Product;
    use crate::q3::foundation::animation::ANIMATION_TOGGLE_BIT;
    use qa_core::identity::{IdentityOwner, ProviderId};
    use qa_core::time::{FramePhase, SourceTime};
    use qa_world::movement::q3::types::q3_animation_state;
    use qa_world::movement::types::{AnimationState, InventoryEntry, Q3UserCommand, WeaponState as FamilyWeaponState};

    use super::*;

    fn test_frame(elapsed: SourceTime) -> FrameContext {
        FrameContext {
            frame: 1,
            time: SourceTime::Milliseconds(100),
            elapsed,
            phase: FramePhase::FrameEntry,
        }
    }

    fn test_actor() -> (OwnedActor, ProviderId) {
        let owner = IdentityOwner::create("test").unwrap();
        let provider = ProviderId::new("q3", "test");
        let owned = owner.owned_actor(&owner.actor(3, 1), provider.clone()).unwrap();
        (owned, provider)
    }

    fn test_command() -> UserCommand {
        UserCommand::Q3(Q3UserCommand {
            server_time_milliseconds: 0,
            angle_words: [0, 0, 0],
            buttons: 0,
            weapon: weapon::MACHINEGUN,
            forward_move: 0,
            right_move: 0,
            up_move: 0,
        })
    }

    fn test_environment(health: f64) -> MovementEnvironment {
        MovementEnvironment {
            client_outputs: None,
            speed_multiplier: None,
            pose: None,
            health,
            flight: false,
            haste: false,
            invulnerable: false,
            gravity_multiplier: 1.0,
        }
    }

    fn q3_weapon(state: &FamilyWeaponState) -> (i32, i32, i32) {
        let FamilyWeaponState::Q3 {
            source_weapon,
            state,
            time_milliseconds,
        } = state
        else {
            panic!("q3 step keeps q3 weapon state");
        };
        (*source_weapon, *state, *time_milliseconds)
    }

    fn q3_torso(state: &AnimationState) -> i32 {
        let AnimationState::Q3 { torso, .. } = state else {
            panic!("q3 step keeps q3 animation");
        };
        *torso
    }

    struct FakeRuntime {
        state: RefCell<Q3ArsenalRuntimeState>,
        gauntlet: bool,
    }

    impl Q3ArsenalRuntimeAccess for FakeRuntime {
        fn read(&self, _actor: &OwnedActor, _execution: Q3Execution) -> Q3ArsenalRuntimeState {
            self.state.borrow().clone()
        }

        fn write(&self, _actor: &OwnedActor, _execution: Q3Execution, state: Q3ArsenalRuntimeState) {
            *self.state.borrow_mut() = state;
        }

        fn gauntlet_hit(&self, _context: &Q3HookContext) -> bool {
            self.gauntlet
        }
    }

    fn hook_fixture() -> (Q3HookContext, Q3ArsenalRuntimeState) {
        let (actor, provider) = test_actor();
        let runtime = q3_spawn_arsenal_runtime(Product::Baseq3, 100.0, 7);
        let context = Q3HookContext {
            input: Q3HookInput {
                actor,
                execution: Q3Execution::Authoritative,
                command: test_command(),
                environment: test_environment(100.0),
            },
            motion: Q3MotionWork {
                pm_type: move_type::NORMAL,
                pm_flags: move_flags::DUCKED,
                event_sequence: 7,
                product: Q3Product::BaseQ3,
            },
            command: Q3HookCommand {
                buttons: command_buttons::ATTACK,
                weapon: weapon::MACHINEGUN,
            },
            frame: test_frame(SourceTime::Milliseconds(8)),
            arsenal: ArsenalState {
                provider: provider.clone(),
                active_weapon: Some("q3:weapon/machinegun".to_string()),
                state: FamilyWeaponState::Q3 {
                    source_weapon: weapon::MACHINEGUN,
                    state: weapon_state::READY,
                    time_milliseconds: 0,
                },
                ammo: vec![
                    InventoryEntry {
                        item: "q3:weapon/gauntlet".to_string(),
                        count: 1.0,
                    },
                    InventoryEntry {
                        item: "q3:weapon/machinegun".to_string(),
                        count: 1.0,
                    },
                    InventoryEntry {
                        item: "q3:ammo/machinegun".to_string(),
                        count: 100.0,
                    },
                ],
            },
            animation: ActorAnimationState {
                provider,
                state: q3_spawn_animation(),
            },
        };
        (context, runtime)
    }

    #[test]
    fn source_movement_hooks_run_firing_weapon_and_torso() {
        let (mut context, runtime) = hook_fixture();
        let hooks = create_q3_source_movement_hooks(FakeRuntime {
            state: RefCell::new(Q3ArsenalRuntimeState {
                respawned: false,
                ..runtime
            }),
            gauntlet: false,
        });
        assert!(hooks.firing(&context));
        context.arsenal.state = FamilyWeaponState::Q3 {
            source_weapon: 99,
            state: weapon_state::READY,
            time_milliseconds: 0,
        };
        assert!(!hooks.firing(&context));
        context.arsenal.state = FamilyWeaponState::Q3 {
            source_weapon: weapon::MACHINEGUN,
            state: weapon_state::READY,
            time_milliseconds: 0,
        };

        let phase = hooks.weapon(&context).unwrap();
        assert_eq!(q3_weapon(&phase.arsenal.state).1, weapon_state::FIRING);
        assert_eq!(phase.movement_flags, move_flags::DUCKED);

        let AnimationState::Q3 { torso, .. } = &mut context.animation.state else {
            panic!("hook animation stays q3");
        };
        *torso = player_animation::TORSO_ATTACK;
        let torso = hooks.torso(&context).unwrap();
        assert_ne!(q3_torso(&torso.animation.state), q3_torso(&context.animation.state));
        context.arsenal.state = FamilyWeaponState::Q3 {
            source_weapon: weapon::MACHINEGUN,
            state: weapon_state::DROPPING,
            time_milliseconds: 0,
        };
        let held = hooks.torso(&context).unwrap();
        assert!(held.effects.is_empty());

        let legs = hooks
            .animation(
                &Q3AnimationRequest::Legs {
                    animation: player_animation::LEGS_RUN,
                    force: true,
                },
                &context,
            )
            .unwrap();
        assert_eq!(
            q3_animation_state(&legs.animation).unwrap().0 & !ANIMATION_TOGGLE_BIT,
            player_animation::LEGS_RUN
        );
    }
}
