//! Movement admission, mover binding, and train-ride integration tests.

#[path = "common/mod.rs"]
mod common;

use std::cell::RefCell;
use std::rc::Rc;

use common::{test_actor, test_identity, test_profile};
use qa_bots::entity_binding::source_mover_bounds_match;
use qa_bots::movement::{
    create_movement_admission, create_movement_route_admission, NavigationPrediction, NavigationPredictionDriver,
    NavigationPredictionLimits,
};
use qa_bots::movement_contract::{
    MovementExecution, MovementInput, MovementProfile, MovementProvider, MovementResult, MovementServices,
    MovementState,
};
use qa_bots::scene::TraceHit;
use qa_bots::train::{navigation_train_ride, NavigationTrainRide};
use qa_bots::types::{
    NavigationEdge, NavigationEntityState, NavigationSource, TrainState, TrainStop, TravelMode, TraversalAdmission,
    TraversalRequest,
};
use qa_core::identity::ProviderId;
use qa_core::math::{vec3, Bounds, Vec3};
use qa_core::time::{FrameContext, FramePhase, SourceTime};

struct StepProvider {
    profile: MovementProfile,
    target: Vec3,
    position: RefCell<Vec3>,
}

impl MovementProvider for StepProvider {
    fn kind(&self) -> qa_bots::movement_contract::MovementKind {
        self.profile.kind
    }

    fn id(&self) -> &ProviderId {
        &self.profile.id
    }

    fn advance(&self, _input: &MovementInput, _services: &MovementServices) -> MovementResult {
        let mut position = self.position.borrow_mut();
        for axis in 0..3 {
            let current = match axis {
                0 => position.x,
                1 => position.y,
                _ => position.z,
            };
            let goal = match axis {
                0 => self.target.x,
                1 => self.target.y,
                _ => self.target.z,
            };
            let stepped = if (goal - current).abs() <= 32.0 {
                goal
            } else if goal > current {
                current + 32.0
            } else {
                current - 32.0
            };
            match axis {
                0 => position.x = stepped,
                1 => position.y = stepped,
                _ => position.z = stepped,
            }
        }
        MovementResult::Active {
            state: MovementState::Q3 { origin: *position },
            ground: TraceHit::World { model: 0 },
            water_level: 0,
        }
    }
}

struct WalkPrediction {
    provider: StepProvider,
    services: MovementServices,
}

impl NavigationPrediction for WalkPrediction {
    fn provider(&self) -> &dyn MovementProvider {
        &self.provider
    }

    fn services(&self) -> &MovementServices {
        &self.services
    }

    fn input(
        &self,
        _previous: Option<&MovementResult>,
        command_index: usize,
        request: &TraversalRequest,
    ) -> MovementInput {
        let _ = request;
        MovementInput {
            kind: self.provider.profile.kind,
            profile: self.provider.profile.clone(),
            frame: FrameContext {
                frame: command_index as i32,
                time: SourceTime::Milliseconds(command_index as i32 * 8),
                elapsed: SourceTime::Milliseconds(8),
                phase: FramePhase::ClientCommand,
            },
            execution: MovementExecution::Prediction,
        }
    }
}

struct WalkDriver {
    profile: MovementProfile,
}

impl NavigationPredictionDriver for WalkDriver {
    fn begin(
        &self,
        request: &TraversalRequest,
        _profile: &qa_bots::types::NavigationProfile,
    ) -> Option<Box<dyn NavigationPrediction>> {
        Some(Box::new(WalkPrediction {
            provider: StepProvider {
                profile: self.profile.clone(),
                target: request.to,
                position: RefCell::new(request.from),
            },
            services: MovementServices {
                numeric: self.profile.numeric,
            },
        }))
    }
}

struct RefusingDriver;

impl NavigationPredictionDriver for RefusingDriver {
    fn begin(
        &self,
        _request: &TraversalRequest,
        _profile: &qa_bots::types::NavigationProfile,
    ) -> Option<Box<dyn NavigationPrediction>> {
        None
    }
}

fn walk_request() -> TraversalRequest {
    TraversalRequest {
        from: vec3(0.0, 0.0, 0.0),
        to: vec3(64.0, 0.0, 0.0),
        mode: TravelMode::Walk,
        hint: None,
        entity: None,
    }
}

#[test]
fn movement_admission_walks_to_goal() {
    let profile = test_profile();
    let driver = Rc::new(WalkDriver {
        profile: profile.movement.clone(),
    });
    let admit = create_movement_admission(driver, NavigationPredictionLimits::default());
    let admission = admit(&walk_request(), &profile).unwrap();
    let TraversalAdmission::Admitted { seconds, trajectory } = admission else {
        panic!("expected admission");
    };
    assert!(seconds > 0.0);
    assert_eq!(trajectory.first(), Some(&vec3(0.0, 0.0, 0.0)));
    assert_eq!(trajectory.last(), Some(&vec3(64.0, 0.0, 0.0)));
}

#[test]
fn movement_admission_reports_refusals() {
    let profile = test_profile();
    let driver = Rc::new(RefusingDriver);
    let session = create_movement_route_admission(driver, &profile, &NavigationPredictionLimits::default()).unwrap();
    let mut session = session;
    let admission = qa_bots::types::NavigationRoutePrediction::admit(&mut session, &walk_request()).unwrap();
    assert!(matches!(admission, TraversalAdmission::Refused { .. }));
}

#[test]
fn movement_admission_rejects_bad_limits() {
    let profile = test_profile();
    let driver = Rc::new(RefusingDriver);
    let limits = NavigationPredictionLimits {
        maximum_seconds: 0.0,
        maximum_commands: 512,
        tolerance: 8.0,
    };
    assert!(create_movement_route_admission(driver, &profile, &limits).is_err());
}

#[test]
fn mover_bounds_match_accounts_for_travel() {
    let linked = Bounds {
        min: vec3(0.0, 0.0, 0.0),
        max: vec3(64.0, 64.0, 64.0),
    };
    let authored = Bounds {
        min: vec3(0.0, 0.0, 100.0),
        max: vec3(64.0, 64.0, 164.0),
    };
    assert!(source_mover_bounds_match(
        &authored,
        &linked,
        vec3(0.0, 0.0, 0.0),
        vec3(0.0, 0.0, 100.0)
    ));
    assert!(!source_mover_bounds_match(
        &authored,
        &linked,
        vec3(0.0, 0.0, 0.0),
        vec3(0.0, 0.0, 50.0)
    ));
}

fn train_state() -> (NavigationEntityState, NavigationEdge) {
    let owner = test_identity();
    let state = NavigationEntityState {
        actor: test_actor(&owner),
        enabled: true,
        locked: false,
        bounds: Bounds {
            min: vec3(-32.0, -32.0, -8.0),
            max: vec3(32.0, 32.0, 8.0),
        },
        velocity: vec3(0.0, 0.0, 0.0),
        destination: None,
        elevator: None,
        train: Some(TrainState {
            origin: vec3(0.0, 0.0, 0.0),
            running: true,
            stops: vec![
                TrainStop {
                    id: 1,
                    origin: vec3(0.0, 0.0, 0.0),
                    next: Some(2),
                    wait: 0.0,
                    teleport: false,
                },
                TrainStop {
                    id: 2,
                    origin: vec3(64.0, 0.0, 0.0),
                    next: None,
                    wait: 0.0,
                    teleport: false,
                },
            ],
        }),
    };
    let edge = NavigationEdge {
        id: 1,
        from: 1,
        to: 2,
        mode: TravelMode::Mover,
        start: vec3(0.0, 0.0, 24.0),
        end: vec3(64.0, 0.0, 24.0),
        travel_seconds: 2.0,
        source_travel_type: 11,
        source_flags: 0,
        hint: None,
        entity: None,
        source: NavigationSource::Aas {
            area: 1,
            reachability: None,
        },
    };
    (state, edge)
}

#[test]
fn train_ride_follows_directed_stops() {
    let (state, edge) = train_state();
    let ride: Option<NavigationTrainRide> = navigation_train_ride(&state, &edge, &test_profile());
    let ride = ride.unwrap();
    assert_eq!(ride.boarding.id, 1);
    assert_eq!(ride.arrival.id, 2);
}

#[test]
fn train_ride_rejects_locked_and_looping_routes() {
    let (mut state, edge) = train_state();
    state.locked = true;
    assert!(navigation_train_ride(&state, &edge, &test_profile()).is_none());

    let (state, mut edge) = train_state();
    edge.end = edge.start;
    assert!(navigation_train_ride(&state, &edge, &test_profile()).is_none());

    let (mut state, edge) = train_state();
    state.train = None;
    assert!(navigation_train_ride(&state, &edge, &test_profile()).is_none());
}
