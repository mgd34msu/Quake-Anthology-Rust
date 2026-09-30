//! Quake III foundation: movement hooks.
//!
//! Donor provenance: `src/content/q3/foundation/movement-hooks.ts`.

use qa_core::identity::OwnedActor;
use qa_core::time::{FrameContext, SourceTime};

// Intra-group imports: sibling modules split from the same flat port.
use crate::q3::foundation::arsenal_mirror::*;
use crate::q3::foundation::mirrors::*;

// ---------------------------------------------------------------------------
// movement-hooks.ts: PM_Firing, PM_Animate, PM_Weapon, PM_TorsoAnimation.
// ---------------------------------------------------------------------------

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
        attack: context.command.buttons & Q3CommandButtons::ATTACK != 0,
        use_holdable: context.command.buttons & Q3CommandButtons::USE_HOLDABLE != 0,
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

/// Selected movement hooks (`Q3MovementHooks`).
pub trait Q3MovementHooks {
    /// Firing test.
    fn firing(&self, context: &Q3HookContext) -> bool;
    /// Animation request.
    fn animation(&self, request: &Q3AnimationRequest, context: &Q3HookContext) -> AnimationStepResult;
    /// Weapon stage.
    fn weapon(&self, context: &Q3HookContext) -> Result<Q3WeaponPhaseResult, Q3FoundationError>;
    /// Torso stage.
    fn torso(&self, context: &Q3HookContext) -> AnimationStepResult;
}

pub(crate) fn hook_animation_context(context: &Q3HookContext) -> Q3AnimationContext {
    let elapsed_ms = match context.frame.elapsed {
        SourceTime::Milliseconds(value) => f64::from(value),
        SourceTime::Seconds(value) => f64::from(value),
    };
    Q3AnimationContext {
        animation: context.animation.clone(),
        dead: context.motion.pm_type >= Q3MoveType::DEAD,
        elapsed_ms,
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
        let Some(item) = q3_weapon_item(context.arsenal.state.source_weapon) else {
            return false;
        };
        match item.ammo {
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

    fn animation(&self, request: &Q3AnimationRequest, context: &Q3HookContext) -> AnimationStepResult {
        run_q3_animation_operation(request, &hook_animation_context(context))
    }

    fn weapon(&self, context: &Q3HookContext) -> Result<Q3WeaponPhaseResult, Q3FoundationError> {
        let previous = self.runtime.read(&context.input.actor, context.input.execution);
        let state = Q3ArsenalRuntimeState {
            respawned: context.motion.pm_flags & Q3MoveFlags::RESPAWNED != 0,
            use_item_held: context.motion.pm_flags & Q3MoveFlags::USE_ITEM_HELD != 0,
            event_sequence: context.motion.event_sequence,
            ..previous
        };
        let result = step_q3_arsenal(
            &WeaponStepInput {
                actor: context.input.actor.clone(),
                frame: context.frame,
                arsenal: context.arsenal.clone(),
                animation: context.animation.clone(),
                environment: context.input.environment,
                gauntlet_hit: self.runtime.gauntlet_hit(context),
            },
            &state,
            &q3_source_arsenal_controls(context),
            None,
        )?;
        self.runtime
            .write(&context.input.actor, context.input.execution, result.runtime.clone());
        let movement_flags = (context.motion.pm_flags & !(Q3MoveFlags::RESPAWNED | Q3MoveFlags::USE_ITEM_HELD))
            | (if result.runtime.respawned {
                Q3MoveFlags::RESPAWNED
            } else {
                0
            })
            | (if result.runtime.use_item_held {
                Q3MoveFlags::USE_ITEM_HELD
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

    fn torso(&self, context: &Q3HookContext) -> AnimationStepResult {
        if context.arsenal.state.state != Q3WeaponPhase::READY {
            return AnimationStepResult {
                animation: context.animation.clone(),
                effects: Vec::new(),
            };
        }
        let animation = if context.arsenal.state.source_weapon == Q3Weapon::GAUNTLET {
            Q3PlayerAnimation::TORSO_STAND2
        } else {
            Q3PlayerAnimation::TORSO_STAND
        };
        run_q3_torso_operation(animation, &hook_animation_context(context), true)
    }
}
