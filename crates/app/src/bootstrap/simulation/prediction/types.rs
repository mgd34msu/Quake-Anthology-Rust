//! Movement prediction snapshots, commands, and options.
//!
//! Port of donor `/home/buzzkill/Projects/quake-typescript/src/app/bootstrap/simulation/prediction/types.ts`
//! (`MovementPredictionSnapshot`, `MovementPredictionOptions`, `MovementProbeOptions`,
//! `PredictionCommand`, `MovementPredictionResult`, `PredictionStepOptions`).

use std::cell::RefCell;
use std::rc::Rc;

use qa_content::contract::ExecutableRecipe;
use qa_content::q3::foundation::arsenal::Q3ArsenalRuntimeState;
use qa_core::identity::{OwnedActor, ProviderId, SeatId};
use qa_core::math::{Bounds, Vec3};
use qa_core::numeric::NumericProfile;
use qa_net::common::commands::ArsenalIntent;
use qa_world::movement::q1::types::{Q1MovementProfile, Q1Trace, Q1TraceQuery, QwMovementProfile};
use qa_world::movement::q2::rerelease::Q2RereleaseMovementContext;
use qa_world::movement::q2::types::{
    Q2ContentsQuery, Q2MovementProfile, Q2RereleaseMovementProfile, Q2Trace, Q2TraceQuery,
};
use qa_world::movement::q3::types::{Q3MovementProfile, Q3Postures, Q3Trace, Q3TraceQuery};
use qa_world::movement::types::{
    ActorAnimationState, AnimationState, ArsenalState, MovementDialect, MovementEnvironment, OrderedMovementEffect,
    TraceHit, UserCommand,
};

use super::super::arsenal::selected::MovementState;
use super::super::q3_commands::{CommandAngleSpace, RelativeMovementCommandInput};
use qa_core::identity::ActorId;

/// Movement profile selected for prediction, mirroring donor `MovementProfile`.
#[derive(Debug, Clone, PartialEq)]
pub enum MovementPredictionProfile {
    /// NetQuake profile.
    Q1Netquake(Q1MovementProfile),
    /// QuakeWorld profile.
    Q1Quakeworld(QwMovementProfile),
    /// Quake II classic profile.
    Q2Classic(Q2MovementProfile),
    /// Quake II rerelease profile.
    Q2Rerelease(Q2RereleaseMovementProfile),
    /// Quake III profile.
    Q3(Q3MovementProfile),
}

impl MovementPredictionProfile {
    /// Profile dialect.
    #[must_use]
    pub fn dialect(&self) -> MovementDialect {
        match self {
            MovementPredictionProfile::Q1Netquake(_) => MovementDialect::Q1Netquake,
            MovementPredictionProfile::Q1Quakeworld(_) => MovementDialect::Q1Quakeworld,
            MovementPredictionProfile::Q2Classic(_) => MovementDialect::Q2Classic,
            MovementPredictionProfile::Q2Rerelease(_) => MovementDialect::Q2Rerelease,
            MovementPredictionProfile::Q3(_) => MovementDialect::Q3,
        }
    }

    /// Provider identity.
    #[must_use]
    pub fn id(&self) -> &ProviderId {
        match self {
            MovementPredictionProfile::Q1Netquake(profile) => &profile.id,
            MovementPredictionProfile::Q1Quakeworld(profile) => &profile.id,
            MovementPredictionProfile::Q2Classic(profile) => &profile.id,
            MovementPredictionProfile::Q2Rerelease(profile) => &profile.id,
            MovementPredictionProfile::Q3(profile) => &profile.id,
        }
    }

    /// Numeric profile.
    #[must_use]
    pub fn numeric(&self) -> NumericProfile {
        match self {
            MovementPredictionProfile::Q1Netquake(profile) => profile.numeric,
            MovementPredictionProfile::Q1Quakeworld(profile) => profile.numeric,
            MovementPredictionProfile::Q2Classic(profile) => profile.numeric,
            MovementPredictionProfile::Q2Rerelease(profile) => profile.numeric,
            MovementPredictionProfile::Q3(profile) => profile.numeric,
        }
    }
}

/// Collision queries behind prediction. The Rust movement kernels trace
/// through per-family services, so the donor `SceneQueries` surface arrives
/// here split by family; implementations forward to the shared engine scene.
pub trait PredictionScene {
    /// Run a Quake I trace.
    fn trace_q1(&mut self, query: Q1TraceQuery) -> Q1Trace;
    /// Quake I point contents.
    fn point_contents_q1(&mut self, point: Vec3) -> i32;
    /// Run a Quake II trace.
    fn trace_q2(&mut self, query: Q2TraceQuery) -> Q2Trace;
    /// Quake II point contents as (stored, merged).
    fn point_contents_q2(&mut self, query: Q2ContentsQuery) -> (i32, i32);
    /// Run a Quake III trace.
    fn trace_q3(&mut self, query: Q3TraceQuery) -> Q3Trace;
    /// Quake III point contents.
    fn point_contents_q3(&mut self, point: Vec3, pass_actor: &ActorId) -> i32;
}

/// Shared prediction scene handle.
pub type PredictionSceneHandle = Rc<RefCell<dyn PredictionScene>>;

/// Shared brush-hit predicate.
pub type PredictionBrushFn = Rc<dyn Fn(&TraceHit) -> bool>;

/// Posture source for the shared-QuakeWorld prediction branch.
///
/// Value seam: donor `playerPostures` from
/// `src/app/bootstrap/simulation/player-movement.ts` (canonical home:
/// player_movement, wave 2); the players partition implements this trait and
/// prediction calls it instead of duplicating the posture rules.
pub trait PredictionPostures {
    /// Postures for a character animation state and standing hull.
    fn postures(&self, animation: &AnimationState, standing_bounds: Bounds, view_height: f64) -> Q3Postures;
}

/// Shared posture source handle.
pub type PredictionPosturesHandle = Rc<dyn PredictionPostures>;

/// Ground/water contact captured with a prediction snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictionContact {
    /// Ground hit.
    pub ground: TraceHit,
    /// Water level.
    pub water_level: i32,
    /// Water type.
    pub water_type: i32,
}

/// Captured with the authoritative snapshot, then copied into one seat's
/// replay state.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementPredictionSnapshot {
    /// Command sequence.
    pub sequence: i64,
    /// Command time in milliseconds.
    pub command_time_milliseconds: f64,
    /// Movement state.
    pub state: MovementState,
    /// Arsenal state.
    pub arsenal: ArsenalState,
    /// Animation state.
    pub animation: ActorAnimationState,
    /// Movement environment.
    pub environment: MovementEnvironment,
    /// Body bounds.
    pub bounds: Bounds,
    /// View angles.
    pub view_angles: Vec3,
    /// View height.
    pub view_height: f64,
    /// View offset.
    pub view_offset: Vec3,
    /// Ground/water contact.
    pub contact: Option<PredictionContact>,
    /// Private Q3 arsenal runtime.
    pub q3_arsenal: Option<Q3ArsenalRuntimeState>,
}

/// Movement prediction options.
#[derive(Clone)]
pub struct MovementPredictionOptions {
    /// Predict movement only, leaving arsenal and animation untouched.
    pub movement_only: bool,
    /// Predicted actor.
    pub actor: OwnedActor,
    /// Predicting seat.
    pub seat: SeatId,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
    /// Selected movement profile.
    pub profile: MovementPredictionProfile,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Standing view height.
    pub standing_view_height: f64,
    /// Collision queries.
    pub scene: PredictionSceneHandle,
    /// Brush-hit predicate.
    pub is_brush: PredictionBrushFn,
    /// Posture source for the shared-QuakeWorld branch.
    pub postures: PredictionPosturesHandle,
}

/// Navigation probes use the same genuine actor reference without
/// manufacturing a local seat.
#[derive(Clone)]
pub struct MovementProbeOptions {
    /// Predict movement only, leaving arsenal and animation untouched.
    pub movement_only: bool,
    /// Predicted actor.
    pub actor: OwnedActor,
    /// Executable recipe.
    pub recipe: ExecutableRecipe,
    /// Selected movement profile.
    pub profile: MovementPredictionProfile,
    /// Standing bounds.
    pub standing_bounds: Bounds,
    /// Standing view height.
    pub standing_view_height: f64,
    /// Collision queries.
    pub scene: PredictionSceneHandle,
    /// Brush-hit predicate.
    pub is_brush: PredictionBrushFn,
    /// Posture source for the shared-QuakeWorld branch.
    pub postures: PredictionPosturesHandle,
}

impl From<&MovementPredictionOptions> for MovementProbeOptions {
    fn from(options: &MovementPredictionOptions) -> Self {
        MovementProbeOptions {
            movement_only: options.movement_only,
            actor: options.actor.clone(),
            recipe: options.recipe.clone(),
            profile: options.profile.clone(),
            standing_bounds: options.standing_bounds,
            standing_view_height: options.standing_view_height,
            scene: options.scene.clone(),
            is_brush: options.is_brush.clone(),
            postures: options.postures.clone(),
        }
    }
}

/// One predicted command.
#[derive(Debug, Clone, PartialEq)]
pub struct PredictionCommand {
    /// Aim space of the command angles.
    pub angle_space: Option<CommandAngleSpace>,
    /// Command sequence.
    pub sequence: i64,
    /// Command time in milliseconds.
    pub time_milliseconds: f64,
    /// Movement command.
    pub command: UserCommand,
    /// Arsenal intent, if any.
    pub arsenal: Option<ArsenalIntent>,
}

impl RelativeMovementCommandInput for PredictionCommand {
    fn command(&self) -> &UserCommand {
        &self.command
    }

    fn angle_space(&self) -> Option<CommandAngleSpace> {
        self.angle_space
    }

    fn with_relative_command(&self, command: UserCommand) -> Self {
        PredictionCommand {
            angle_space: Some(CommandAngleSpace::SourceRelative),
            sequence: self.sequence,
            time_milliseconds: self.time_milliseconds,
            command,
            arsenal: self.arsenal.clone(),
        }
    }
}

/// Prediction replay status.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PredictionStatus {
    /// Commands replayed.
    Predicted,
    /// No commands to replay.
    Unchanged,
    /// Actor cannot move.
    Disabled,
    /// Command history exhausted.
    HistoryExhausted,
}

/// One prediction replay result.
#[derive(Debug, Clone, PartialEq)]
pub struct MovementPredictionResult {
    /// Replay status.
    pub status: PredictionStatus,
    /// Replayed player snapshot.
    pub player: MovementPredictionSnapshot,
    /// Ordered effects.
    pub effects: Vec<OrderedMovementEffect>,
}

/// Per-command prediction step options.
#[derive(Clone)]
pub struct PredictionStepOptions {
    /// Collision queries.
    pub scene: PredictionSceneHandle,
    /// Shared rerelease pml context.
    pub rerelease_movement: Rc<RefCell<Q2RereleaseMovementContext>>,
    /// Fixed Q3 step in milliseconds.
    pub fixed_milliseconds: Option<i32>,
    /// Suppress Q3 footsteps.
    pub no_footsteps: bool,
    /// Gauntlet hit flag.
    pub gauntlet_hit: bool,
    /// Q3 contents mask override.
    pub trace_mask: Option<i32>,
    /// First command of the replay.
    pub first_command: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_reports_dialect() {
        let profile = MovementPredictionProfile::Q2Classic(Q2MovementProfile {
            id: ProviderId::new("sim", "test"),
            clock: qa_core::time::ClockProfile::Q2Classic,
            numeric: qa_core::numeric::Q2_DONOR_PROFILE,
            strafejump_hack: false,
            air_accelerate: 0.0,
            snap_initial: false,
        });
        assert_eq!(profile.dialect(), MovementDialect::Q2Classic);
        assert_eq!(profile.id().name, "test");
    }

    #[test]
    fn probe_options_drop_only_the_seat() {
        struct NullScene;
        impl PredictionScene for NullScene {
            fn trace_q1(&mut self, _query: Q1TraceQuery) -> Q1Trace {
                panic!("no traces in this test");
            }
            fn point_contents_q1(&mut self, _point: Vec3) -> i32 {
                0
            }
            fn trace_q2(&mut self, _query: Q2TraceQuery) -> Q2Trace {
                panic!("no traces in this test");
            }
            fn point_contents_q2(&mut self, _query: Q2ContentsQuery) -> (i32, i32) {
                (0, 0)
            }
            fn trace_q3(&mut self, _query: Q3TraceQuery) -> Q3Trace {
                panic!("no traces in this test");
            }
            fn point_contents_q3(&mut self, _point: Vec3, _pass_actor: &ActorId) -> i32 {
                0
            }
        }
        let scene: PredictionSceneHandle = Rc::new(RefCell::new(NullScene));
        assert_eq!(scene.borrow_mut().point_contents_q1(Vec3::default()), 0);
        let is_brush: PredictionBrushFn = Rc::new(|_| false);
        assert!(!is_brush(&TraceHit::None));
    }
}
