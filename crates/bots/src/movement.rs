//! Movement probes invoke the existing authoritative movement
//! implementation in prediction mode, ported from
//! `src/bots/navigation/movement.ts`.

use std::rc::Rc;

use crate::error::BotsError;
use crate::helpers::distance;
use crate::movement_contract::{MovementInput, MovementProvider, MovementResult, MovementServices};
use crate::types::{NavigationProfile, NavigationRoutePrediction, TravelMode, TraversalAdmission, TraversalRequest};

/// Prediction session behind one route admission: provider, services,
/// and the input builder. Builds independent prediction state; never
/// mutates or dispatches contacts to the live actor.
pub trait NavigationPrediction {
    /// Selected movement provider.
    fn provider(&self) -> &dyn MovementProvider;
    /// Movement services.
    fn services(&self) -> &MovementServices;
    /// Build the next prediction input.
    fn input(
        &self,
        previous: Option<&MovementResult>,
        command_index: usize,
        request: &TraversalRequest,
    ) -> MovementInput;
}

/// Builds prediction sessions for traversal requests.
pub trait NavigationPredictionDriver {
    /// Begin a prediction session, or `None` when the selected movement
    /// cannot predict the request.
    fn begin(&self, request: &TraversalRequest, profile: &NavigationProfile) -> Option<Box<dyn NavigationPrediction>>;
}

fn step(input: &MovementInput, prediction: &dyn NavigationPrediction) -> Result<MovementResult, BotsError> {
    if input.execution != crate::movement_contract::MovementExecution::Prediction {
        return Err(BotsError::PredictionMode);
    }
    if prediction.provider().kind() == input.kind {
        return Ok(prediction.provider().advance(input, prediction.services()));
    }
    Err(BotsError::InputProviderMismatch)
}

/// Prediction limits.
#[derive(Debug, Clone, PartialEq)]
pub struct NavigationPredictionLimits {
    /// Maximum seconds per admission.
    pub maximum_seconds: f64,
    /// Maximum commands per admission.
    pub maximum_commands: usize,
    /// Arrival tolerance in units.
    pub tolerance: f64,
}

impl Default for NavigationPredictionLimits {
    fn default() -> Self {
        Self {
            maximum_seconds: 8.0,
            maximum_commands: 512,
            tolerance: 8.0,
        }
    }
}

/// Route admission over a prediction driver.
pub struct MovementRouteAdmission {
    driver: Rc<dyn NavigationPredictionDriver>,
    profile: NavigationProfile,
    maximum_seconds: f64,
    maximum_commands: usize,
    tolerance: f64,
    prediction: Option<Box<dyn NavigationPrediction>>,
    previous: Option<MovementResult>,
    sequence: usize,
    failed: bool,
}

/// Build a detached route-prediction session over a driver.
pub fn create_movement_route_admission(
    driver: Rc<dyn NavigationPredictionDriver>,
    profile: &NavigationProfile,
    limits: &NavigationPredictionLimits,
) -> Result<MovementRouteAdmission, BotsError> {
    if !limits.maximum_seconds.is_finite()
        || limits.maximum_seconds <= 0.0
        || limits.maximum_commands == 0
        || !limits.tolerance.is_finite()
        || limits.tolerance <= 0.0
    {
        return Err(BotsError::PredictionLimits);
    }
    Ok(MovementRouteAdmission {
        driver,
        profile: profile.clone(),
        maximum_seconds: limits.maximum_seconds,
        maximum_commands: limits.maximum_commands,
        tolerance: limits.tolerance,
        prediction: None,
        previous: None,
        sequence: 0,
        failed: false,
    })
}

impl NavigationRoutePrediction for MovementRouteAdmission {
    fn admit(&mut self, request: &TraversalRequest) -> Result<TraversalAdmission, BotsError> {
        if self.failed {
            return Err(BotsError::PredictionPoisoned);
        }
        if self.prediction.is_none() {
            self.prediction = self.driver.begin(request, &self.profile);
        }
        let Some(prediction) = self.prediction.as_ref() else {
            return Ok(TraversalAdmission::Refused {
                reason: format!("selected movement cannot predict {}", request.mode.as_str()),
            });
        };
        if prediction.provider().kind() != self.profile.movement.kind
            || prediction.provider().id() != &self.profile.movement.id
        {
            return Err(BotsError::PredictorMismatch);
        }
        let mut seconds = 0.0;
        let mut trajectory = vec![request.from];
        let mut command = 0;
        while command < self.maximum_commands && seconds < self.maximum_seconds {
            let input = prediction.input(self.previous.as_ref(), self.sequence, request);
            self.sequence += 1;
            if input.profile != self.profile.movement {
                return Err(BotsError::PredictionProfileChanged);
            }
            let elapsed = match input.frame.elapsed {
                qa_core::time::SourceTime::Milliseconds(value) => f64::from(value) / 1000.0,
                qa_core::time::SourceTime::Seconds(value) => f64::from(value),
            };
            if !elapsed.is_finite() || elapsed <= 0.0 {
                return Err(BotsError::PredictionStalled);
            }
            let result = step(&input, &**prediction)?;
            if matches!(result, MovementResult::ActorRemoved) {
                self.failed = true;
                return Ok(TraversalAdmission::Refused {
                    reason: "movement removed the predicted actor".to_string(),
                });
            }
            seconds += elapsed;
            let (origin, grounded) = match &result {
                MovementResult::Active {
                    state,
                    ground,
                    water_level,
                } => (
                    state.origin(),
                    !matches!(ground, crate::scene::TraceHit::None)
                        || *water_level > 0
                        || request.mode == TravelMode::Ladder,
                ),
                MovementResult::ActorRemoved => unreachable!(),
            };
            trajectory.push(origin);
            self.previous = Some(result);
            if distance(origin, request.to) <= self.tolerance && grounded {
                return Ok(TraversalAdmission::Admitted { seconds, trajectory });
            }
            command += 1;
        }
        self.failed = true;
        Ok(TraversalAdmission::Refused {
            reason: format!(
                "selected movement did not reach the landing within {}s/{} commands",
                self.maximum_seconds, self.maximum_commands
            ),
        })
    }
}

/// Build single-traversal admission over a driver.
pub fn create_movement_admission(
    driver: Rc<dyn NavigationPredictionDriver>,
    limits: NavigationPredictionLimits,
) -> impl Fn(&TraversalRequest, &NavigationProfile) -> Result<TraversalAdmission, BotsError> {
    move |request, profile| create_movement_route_admission(driver.clone(), profile, &limits)?.admit(request)
}
